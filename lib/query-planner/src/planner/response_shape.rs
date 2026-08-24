//! The compiled shape of a subgraph fetch's response.
//!
//! The planner already knows, before a single byte comes back, which response keys a fetch
//! will return and what the router intends to do with each one. `ResponseShape` records
//! that once so the executor stops rediscovering it per response: fields the router never
//! inspects are copied straight out of the subgraph bytes instead of being parsed into a
//! `Value` and serialized again.
//!
//! This replaces the older `CustomScalarPaths` trie, which only marked custom scalars and
//! resolved keys through a `BTreeMap<String, _>` — a lookup that cost ~16ns on *every* key
//! of any fetch containing a custom scalar anywhere, swamping what the passthrough saved.

use crate::{
    ast::{selection_item::SelectionItem, selection_set::SelectionSet},
    state::supergraph_state::{SupergraphDefinition, SupergraphState, TypeNode},
};

use super::fetch::{selections::FetchStepSelections, state::MultiTypeFetchStep};

const TYPENAME_FIELD_NAME: &str = "__typename";

/// The shape of one position in a subgraph response.
///
/// *Every* response key is listed, in selection order, so key resolution is a cursor hit
/// rather than a map lookup, and so a field's index can serve as its slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResponseShape {
    /// Every response key the fetch selects at this position, in selection order. The index
    /// of a field is its slot.
    pub fields: Vec<ResponseShapeField>,
    /// Copy the value at this position verbatim out of the response bytes.
    pub raw: bool,
    /// Nothing in this subtree is a passthrough.
    ///
    /// Informational only. It used to let the deserializer skip per-key lookups for a whole
    /// subtree, but slot addressing needs the shape for every object, so there is no longer a
    /// lookup-free path to take. Kept because it is the natural way to ask "does this fetch
    /// pass anything through", which the shape tests do.
    pub inert: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResponseShapeField {
    pub key: String,
    pub shape: ResponseShape,
}

/// An empty shape is inert: nothing described, nothing to pass through, plain parsing.
impl Default for ResponseShape {
    fn default() -> Self {
        ResponseShape {
            fields: Vec::new(),
            raw: false,
            inert: true,
        }
    }
}

/// `__typename` is reserved at slot 0 of every position reachable from the client
/// operation, so readers that have no shape in hand can still find it.
pub const TYPENAME_SLOT: usize = 0;

impl ResponseShape {
    /// Resolves a response key to the shape of its value.
    ///
    /// Subgraphs return fields in the order the fetch asked for them, so `cursor` — the
    /// slot after the previous hit — is almost always right, making the common case a
    /// single string compare.
    ///
    /// ponytail: linear fallback. Selection sets at one position are small (typically
    /// under 20 keys) and a packed `Vec` scan beats a map there; switch to binary search
    /// if a wide-object bench ever says otherwise.
    #[inline]
    pub fn resolve(&self, cursor: &mut usize, key: &str) -> Option<&ResponseShape> {
        self.resolve_slot(cursor, key)
            .map(|slot| &self.fields[slot].shape)
    }

    /// The slot a response key occupies, or `None` for a key the plan never asked for.
    #[inline]
    pub fn resolve_slot(&self, cursor: &mut usize, key: &str) -> Option<usize> {
        if let Some(field) = self.fields.get(*cursor) {
            if field.key == key {
                let slot = *cursor;
                *cursor += 1;
                return Some(slot);
            }
        }

        let slot = self.fields.iter().position(|field| field.key == key)?;
        *cursor = slot + 1;
        Some(slot)
    }

    /// The slot for `key`, ignoring arrival order. For plan-time compilation, not parsing.
    pub fn slot_of(&self, key: &str) -> Option<usize> {
        self.fields.iter().position(|field| field.key == key)
    }

    pub fn child(&self, slot: usize) -> Option<&ResponseShape> {
        self.fields.get(slot).map(|field| &field.shape)
    }

    /// Test/plumbing helper: marks `path` as a passthrough, creating nodes as needed.
    pub fn insert_raw_path<I, S>(&mut self, path: I)
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut node = self;
        node.inert = false;
        for segment in path {
            let key = segment.as_ref();
            let idx = match node.fields.iter().position(|field| field.key == key) {
                Some(idx) => idx,
                None => {
                    node.fields.push(ResponseShapeField {
                        key: key.to_string(),
                        shape: ResponseShape::default(),
                    });
                    node.fields.len() - 1
                }
            };
            node = &mut node.fields[idx].shape;
            node.inert = false;
        }
        node.raw = true;
    }
}

/// Whether a field's value is worth handing to the client as untouched subgraph bytes.
///
/// Only leaves qualify. A composite subtree is reshaped on the way out — aliases, injected
/// `__typename`, client field order, `@skip`/`@include`, type guards — so its bytes would
/// not match what projection has to emit.
///
/// Being *safe* to pass through is not enough, it also has to be *worth* it. Passthrough
/// saves per byte of value (no number parsing or reformatting, no escape scan on the way
/// in or out), but marking anything raw forces that object through the shaped visitor,
/// which costs per key: it has to read the key before it can pick the child shape, so it
/// gives up the fused `next_entry` the plain visitor gets. Measured on a 200-object
/// deserialize+project round trip:
///
/// | payload               | structured | raw      |
/// |-----------------------|-----------:|---------:|
/// | objects of scalars    |    91.7 us | 112.5 us |
/// | objects of numbers    |   212.4 us | 231.0 us |
/// | objects of leaf lists |   171.1 us |  93.1 us |
///
/// So the rule is aggregates only: lists of leaves, and custom scalars, whose values are
/// JSON blobs. A single builtin scalar has too little payload to repay the per-key cost.
fn is_raw_eligible(
    supergraph: &SupergraphState,
    response_key: &str,
    type_name: &str,
    field_type: &TypeNode,
) -> bool {
    // `__typename` is read back as a `&str` by projection and by type-condition traversal.
    if response_key == TYPENAME_FIELD_NAME {
        return false;
    }

    match supergraph.definitions.get(type_name) {
        // Enums are matched against `Value::String` by authorization's enum filtering, which
        // a `RawJson` would silently skip. An enum value is always a JSON string, so parsing
        // it structurally loses nothing.
        Some(SupergraphDefinition::Enum(_)) => false,
        // Custom scalars are always a passthrough, which is both the faster and the only
        // lossless choice: their value can be an arbitrary JSON object, and an object at a
        // leaf position has no shape to be placed into. (This is also exactly what the
        // `CustomScalarPaths` this replaced did, `@cost` weight included.)
        Some(SupergraphDefinition::Scalar(_)) => true,
        // Builtin scalars carry no definition of their own, and their values are never
        // objects, so structural parsing loses nothing. Passthrough only repays its per-key
        // cost for a list, and only when nothing inside that list is non-null: a `null` at a
        // non-null position has to null out the enclosing list, and raw bytes would emit it
        // as-is.
        None if supergraph.is_scalar_type(type_name) => {
            field_type.is_list()
                && !has_non_null_inside_a_list(field_type)
                && demand_control_weight(supergraph, type_name) == 0
        }
        _ => false,
    }
}

/// Whether a `null` anywhere inside this value would have to propagate.
///
/// The field's own outermost non-null does not count: a `null` for the whole value is
/// normalized back to `Value::Null` when it is read, so the caller still propagates it. Only
/// non-null positions *inside* a list need the structure to be preserved.
fn has_non_null_inside_a_list(field_type: &TypeNode) -> bool {
    fn walk(node: &TypeNode) -> bool {
        match node {
            TypeNode::NonNull(_) => true,
            TypeNode::List(inner) => walk(inner),
            TypeNode::Named(_) => false,
        }
    }

    match field_type.unwrap_non_null() {
        TypeNode::List(inner) => walk(inner),
        _ => false,
    }
}

/// A `@cost` weight makes a list's length observable to demand control, and raw bytes hide
/// the length. With weight 0 — the default for scalars and enums — the length is irrelevant
/// and the cost stays exact.
fn demand_control_weight(supergraph: &SupergraphState, type_name: &str) -> u64 {
    match supergraph.definitions.get(type_name) {
        Some(SupergraphDefinition::Scalar(scalar)) => {
            scalar.cost.as_ref().map(|cost| cost.weight).unwrap_or(0)
        }
        Some(SupergraphDefinition::Enum(def)) => {
            def.cost.as_ref().map(|cost| cost.weight).unwrap_or(0)
        }
        _ => 0,
    }
}

/// Build-time mirror of `ResponseShape`, in first-seen selection order.
///
/// `raw_eligible` / `structured` track the two ways one response key can be reached — an
/// interface or union can select the same key on several branches, and the key is only a
/// passthrough if *every* branch agrees it is one.
#[derive(Debug, Default)]
struct ShapeBuilder {
    fields: Vec<(String, ShapeBuilder)>,
    raw_eligible: bool,
    structured: bool,
}

impl ShapeBuilder {
    fn child(&mut self, key: &str) -> &mut ShapeBuilder {
        let idx = match self.fields.iter().position(|(k, _)| k == key) {
            Some(idx) => idx,
            None => {
                self.fields.push((key.to_string(), ShapeBuilder::default()));
                self.fields.len() - 1
            }
        };
        &mut self.fields[idx].1
    }

    /// Always describes the node in full — slot addressing needs every key — and marks a
    /// subtree `inert` when nothing under it is a passthrough, so the deserializer can still
    /// skip per-key lookups there.
    fn build(self) -> ResponseShape {
        if self.raw_eligible && !self.structured && self.fields.is_empty() {
            return ResponseShape {
                fields: Vec::new(),
                raw: true,
                inert: false,
            };
        }

        let fields: Vec<ResponseShapeField> = self
            .fields
            .into_iter()
            .map(|(key, child)| ResponseShapeField {
                key,
                shape: child.build(),
            })
            .collect();

        let inert = fields.iter().all(|field| field.shape.inert);

        ResponseShape {
            fields,
            raw: false,
            inert,
        }
    }
}

fn collect(
    builder: &mut ShapeBuilder,
    parent_type_name: &str,
    selections: &SelectionSet,
    supergraph: &SupergraphState,
) {
    let Some(parent_def) = supergraph.definitions.get(parent_type_name) else {
        return;
    };
    let parent_fields = parent_def.fields();

    for item in &selections.items {
        match item {
            SelectionItem::Field(field) => {
                let response_key = field.selection_identifier();

                // `__typename` has no field definition, but it still occupies a response
                // key and the cursor has to account for it.
                let Some(field_def) = parent_fields.get(&field.name) else {
                    if field.name == TYPENAME_FIELD_NAME {
                        builder.child(response_key).structured = true;
                    }
                    continue;
                };

                let field_type_name = field_def.field_type.inner_type();
                let child = builder.child(response_key);

                if field.selections.is_empty()
                    && is_raw_eligible(
                        supergraph,
                        response_key,
                        field_type_name,
                        &field_def.field_type,
                    )
                {
                    child.raw_eligible = true;
                } else {
                    child.structured = true;
                    if !field.selections.is_empty() {
                        collect(child, field_type_name, &field.selections, supergraph);
                    }
                }
            }
            SelectionItem::InlineFragment(fragment) => {
                collect(
                    builder,
                    &fragment.type_condition,
                    &fragment.selections,
                    supergraph,
                );
            }
            SelectionItem::FragmentSpread(_) => {}
        }
    }
}

pub fn response_shape_from_fetch_output(
    output: &FetchStepSelections<MultiTypeFetchStep>,
    supergraph: &SupergraphState,
    entities_root_key: Option<&str>,
) -> ResponseShape {
    let mut root = ShapeBuilder::default();

    for (type_name, selection_set) in output.iter_selections() {
        let builder = match entities_root_key {
            Some(key) => {
                let entities = root.child(key);
                entities.structured = true;
                entities
            }
            None => &mut root,
        };
        collect(builder, type_name, selection_set, supergraph);
    }

    root.build()
}

/// Entity calls are always built from top-level `... on Type` fragments, so there is no
/// starting type name to hand to `collect` — each fragment supplies its own.
pub fn response_shape_for_entities_selection(
    selection_set: &SelectionSet,
    supergraph: &SupergraphState,
) -> ResponseShape {
    let mut root = ShapeBuilder::default();

    for item in &selection_set.items {
        if let SelectionItem::InlineFragment(fragment) = item {
            collect(
                &mut root,
                &fragment.type_condition,
                &fragment.selections,
                supergraph,
            );
        }
    }

    root.build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        planner::fetch::{selections::FetchStepSelections, state::SingleTypeFetchStep},
        utils::parsing::parse_schema,
    };

    fn selection_set(source: &str) -> SelectionSet {
        graphql_tools::parser::parse_query(source)
            .unwrap()
            .into_static()
            .definitions
            .into_iter()
            .find_map(|definition| match definition {
                graphql_tools::parser::query::Definition::Operation(
                    graphql_tools::parser::query::OperationDefinition::SelectionSet(set),
                ) => Some(set.into()),
                _ => None,
            })
            .expect("selection set")
    }

    /// Builds a two-branch fetch output where both branches share one response key.
    fn two_branch_shape(
        sdl: &str,
        branch_a: (&str, &str),
        branch_b: (&str, &str),
    ) -> Option<ResponseShape> {
        // Tests read "no shape" as "nothing to pass through", which is now `inert`.
        let schema = parse_schema(sdl);
        let supergraph = SupergraphState::new(&schema);

        let mut output = FetchStepSelections::<SingleTypeFetchStep>::new_empty().into_multi_type();
        output.declare_known_type(branch_a.0);
        output.declare_known_type(branch_b.0);
        *output.selections_for_definition_mut(branch_a.0).unwrap() = selection_set(branch_a.1);
        *output.selections_for_definition_mut(branch_b.0).unwrap() = selection_set(branch_b.1);

        let shape = response_shape_from_fetch_output(&output, &supergraph, Some("_entities"));
        (!shape.inert).then_some(shape)
    }

    fn field<'a>(shape: &'a ResponseShape, key: &str) -> &'a ResponseShape {
        &shape
            .fields
            .iter()
            .find(|f| f.key == key)
            .unwrap_or_else(|| panic!("missing field {key} in {shape:?}"))
            .shape
    }

    const LEAF_SDL: &str = r#"
        scalar JSONBlob
        enum Color { RED }
        type TypeA { meta: JSONBlob }
        type TypeB { meta: String }
        type TypeC { meta: Color }
        type TypeD { meta: Nested }
        type TypeE { meta: [String] }
        type Nested { id: ID }
        type Query { root: String }
    "#;

    #[test]
    fn a_plain_scalar_branch_keeps_a_shared_key_structured() {
        // A single builtin scalar is not worth a passthrough, so the two branches disagree
        // and the shared key stays structured.
        assert!(
            two_branch_shape(LEAF_SDL, ("TypeA", "{ meta }"), ("TypeB", "{ meta }")).is_none()
        );
    }

    #[test]
    fn leaf_lists_and_custom_scalars_are_passthrough() {
        let shape =
            two_branch_shape(LEAF_SDL, ("TypeA", "{ meta }"), ("TypeE", "{ meta }")).expect("shape");
        assert!(
            field(field(&shape, "_entities"), "meta").raw,
            "a custom scalar and a list of leaves are both aggregates worth passing through"
        );
    }

    #[test]
    fn a_list_with_non_null_items_keeps_its_structure() {
        // A `null` at a non-null position has to null out the enclosing list. Raw bytes would
        // emit it as-is, so these lists cannot be passed through.
        let sdl = r#"
            type Query { root: String }
            type Nullable { v: [Int] }
            type NonNullItems { v: [Int!] }
            type NonNullField { v: [Int]! }
            type NestedNonNull { v: [[Int!]] }
            type NestedNullable { v: [[Int]] }
        "#;

        let raw_for = |type_name: &str| {
            two_branch_shape(sdl, (type_name, "{ v }"), (type_name, "{ v }"))
                .map(|shape| field(field(&shape, "_entities"), "v").raw)
                .unwrap_or(false)
        };

        assert!(raw_for("Nullable"), "[Int] has nothing to propagate");
        assert!(
            raw_for("NonNullField"),
            "[Int]! only makes the field itself non-null, which is handled where it is read"
        );
        assert!(raw_for("NestedNullable"), "[[Int]] has nothing to propagate");
        assert!(!raw_for("NonNullItems"), "[Int!] needs its structure");
        assert!(!raw_for("NestedNonNull"), "[[Int!]] needs its structure");
    }

    #[test]
    fn a_single_builtin_scalar_is_not_worth_a_passthrough() {
        let sdl = r#"
            type Query { root: String }
            type TypeA { name: String }
            type TypeB { name: String }
        "#;
        assert!(
            two_branch_shape(sdl, ("TypeA", "{ name }"), ("TypeB", "{ name }")).is_none(),
            "per-key shaped-visitor cost outweighs what a single scalar saves"
        );
    }

    #[test]
    fn enum_branch_keeps_a_shared_key_structured() {
        // Enums are matched against `Value::String` by authorization's enum filtering.
        let shape = two_branch_shape(LEAF_SDL, ("TypeA", "{ meta }"), ("TypeC", "{ meta }"));
        assert!(
            shape.is_none(),
            "an enum branch must keep the shared key structured, leaving nothing to pass through"
        );
    }

    #[test]
    fn composite_branch_keeps_a_shared_key_structured() {
        assert!(
            two_branch_shape(LEAF_SDL, ("TypeA", "{ meta }"), ("TypeD", "{ meta { id } }"))
                .is_none(),
            "a composite branch must keep the shared key structured"
        );
    }

    #[test]
    fn cost_weighted_builtin_list_is_not_passthrough() {
        // A `@cost` weight makes list length observable to demand control, which raw bytes
        // hide. Custom scalars are exempt: they can hold objects, so passthrough is the only
        // lossless option for them.
        let sdl = r#"
            directive @cost(weight: Int!) on SCALAR | FIELD_DEFINITION
            enum Pricey @cost(weight: 5) { A }
            type TypeA { a: [Pricey!]! }
            type TypeB { b: [String] }
            type Query { root: String }
        "#;
        let shape = two_branch_shape(sdl, ("TypeA", "{ a }"), ("TypeB", "{ b }")).expect("shape");
        let entities = field(&shape, "_entities");
        assert!(
            !field(entities, "a").raw,
            "@cost-weighted list must stay structured so demand control still sees its length"
        );
        assert!(field(entities, "b").raw);
    }

    #[test]
    fn a_custom_scalar_is_always_a_passthrough() {
        // Its value can be an arbitrary JSON object, and a leaf position has no shape to
        // place an object into, so raw bytes are the only lossless representation.
        let sdl = r#"
            directive @cost(weight: Int!) on SCALAR | FIELD_DEFINITION
            scalar Blob @cost(weight: 9)
            type TypeA { a: Blob }
            type TypeB { a: Blob }
            type Query { root: String }
        "#;
        let shape = two_branch_shape(sdl, ("TypeA", "{ a }"), ("TypeB", "{ a }")).expect("shape");
        assert!(field(field(&shape, "_entities"), "a").raw);
    }

    #[test]
    fn shape_lists_every_key_so_the_cursor_stays_aligned() {
        let sdl = r#"
            scalar JSONBlob
            type Query { root: String }
            type TypeA { id: ID, meta: JSONBlob, nested: Nested }
            type Nested { id: ID }
            type TypeB { id: ID }
        "#;
        // Every key is listed even though only `meta` is a passthrough, so the deserializer's
        // cursor stays aligned with the order the subgraph answers in.
        let shape = two_branch_shape(
            sdl,
            ("TypeA", "{ __typename id meta nested { id } }"),
            ("TypeB", "{ id }"),
        )
        .expect("shape");
        let entities = field(&shape, "_entities");
        let keys: Vec<&str> = entities.fields.iter().map(|f| f.key.as_str()).collect();
        assert_eq!(keys, ["__typename", "id", "meta", "nested"]);
        assert!(!field(entities, "__typename").raw);
        assert!(field(entities, "meta").raw);
    }

    #[test]
    fn inert_subtree_is_pruned_entirely() {
        let sdl = r#"
            enum Color { RED }
            type Query { root: String }
            type TypeA { color: Color }
            type TypeB { color: Color }
        "#;
        assert!(
            two_branch_shape(sdl, ("TypeA", "{ color }"), ("TypeB", "{ color }")).is_none(),
            "nothing to pass through must prune the shape so the deserializer stays lookup-free"
        );
    }

    #[test]
    fn resolve_uses_the_cursor_then_falls_back_to_a_scan() {
        let mut shape = ResponseShape::default();
        shape.insert_raw_path(["a"]);
        shape.insert_raw_path(["b"]);
        shape.insert_raw_path(["c"]);

        let mut cursor = 0;
        assert!(shape.resolve(&mut cursor, "a").is_some());
        assert_eq!(cursor, 1);
        // Out of order: the scan finds it and re-anchors the cursor after the hit.
        assert!(shape.resolve(&mut cursor, "c").is_some());
        assert_eq!(cursor, 3);
        assert!(shape.resolve(&mut cursor, "b").is_some());
        assert_eq!(cursor, 2);
        assert!(shape.resolve(&mut cursor, "missing").is_none());
    }
}
