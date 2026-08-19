//! Merges every fetch's response shape into one description of the *merged* response tree.
//!
//! Each fetch is then pointed at the node for its own position, so two responses that land
//! in the same place are deserialized against the same field order. That is what makes a
//! field's index usable as a slot: merging becomes a positional zip, and every later lookup
//! becomes an index instead of a key comparison.
//!
//! The slot layout is never stored in the data. Every consumer — projection, merge,
//! traversal, `requires` — already knows which position it is looking at, so the layout is
//! implied by the position and the values can be a bare dense vector.

use crate::{
    ast::{
        operation::OperationDefinition, selection_item::SelectionItem, selection_set::SelectionSet,
    },
    planner::{
        plan_nodes::{
            FetchNodePathSegment, FetchRewrite, FlattenNodePath, FlattenNodePathSegment, PlanNode,
        },
        response_shape::{ResponseShape, ResponseShapeField},
        slot_path::{compile_flatten_path, compile_requires, compile_rewrites},
    },
};

pub const ENTITIES_KEY: &str = "_entities";
const TYPENAME_KEY: &str = "__typename";

/// Mutable mirror of `ResponseShape` used while accumulating fetches.
#[derive(Debug, Default)]
struct MergedNode {
    fields: Vec<(String, MergedNode)>,
    raw: bool,
}

impl MergedNode {
    fn child(&mut self, key: &str) -> &mut MergedNode {
        let idx = match self.fields.iter().position(|(k, _)| k == key) {
            Some(idx) => idx,
            None => {
                self.fields.push((key.to_string(), MergedNode::default()));
                self.fields.len() - 1
            }
        };
        &mut self.fields[idx].1
    }

    /// Absorbs the client's own selection set. Fetch outputs cover what subgraphs return,
    /// but not fields the router answers itself — introspection builds `__schema` straight
    /// into the response tree — and the client's response keys are what projection reads.
    fn absorb_selections(&mut self, selections: &SelectionSet) {
        // `__typename` is reserved at slot 0 of every position the client can see, whether
        // or not the client asked for it. Projection and type-condition traversal both need
        // to read it, and neither has the plan's shape in hand — pinning the slot is what
        // lets them look it up without one.
        self.child(TYPENAME_KEY);

        for item in &selections.items {
            match item {
                SelectionItem::Field(field) => {
                    let child = self.child(field.selection_identifier());
                    if !field.selections.is_empty() {
                        child.absorb_selections(&field.selections);
                    }
                }
                // A polymorphic position is one node holding the union of every branch.
                SelectionItem::InlineFragment(fragment) => {
                    self.absorb_selections(&fragment.selections)
                }
                SelectionItem::FragmentSpread(_) => {}
            }
        }
    }

    /// Absorbs the response keys a `requires` selection reads, and the keys rewrites move
    /// values into. Neither shows up in a fetch's output, but both address the merged tree,
    /// so they need slots of their own.
    fn absorb_requires(&mut self, selections: &SelectionSet) {
        for item in &selections.items {
            match item {
                SelectionItem::Field(field) => {
                    let child = self.child(&field.name);
                    if !field.selections.is_empty() {
                        child.absorb_requires(&field.selections);
                    }
                }
                SelectionItem::InlineFragment(fragment) => {
                    self.child(TYPENAME_KEY);
                    self.absorb_requires(&fragment.selections);
                }
                SelectionItem::FragmentSpread(_) => {}
            }
        }
    }

    fn absorb_rewrite_targets(&mut self, rewrites: &[FetchRewrite]) {
        for rewrite in rewrites {
            let (path, target) = match rewrite {
                FetchRewrite::KeyRenamer(renamer) => {
                    (renamer.path.as_slice(), Some(renamer.rename_key_to.as_str()))
                }
                FetchRewrite::ValueSetter(setter) => (setter.path.as_slice(), None),
            };

            let mut node = &mut *self;
            let last = if target.is_some() { 1 } else { 0 };
            for segment in &path[..path.len().saturating_sub(last)] {
                match segment {
                    FetchNodePathSegment::Key(key) => node = node.child(key),
                    FetchNodePathSegment::TypenameEquals(_) => {
                        node.child(TYPENAME_KEY);
                    }
                }
            }
            if let Some(target) = target {
                node.child(target);
            }
        }
    }

    /// Absorbs one fetch's shape at this position. Field order is first-seen across all
    /// fetches, so a later fetch's extra keys append rather than shifting existing slots.
    fn absorb(&mut self, shape: &ResponseShape) {
        // A key is only a passthrough if *every* fetch that produces it agrees. One fetch
        // wanting it structured has to win, or the structured reader would find raw bytes.
        if shape.raw && shape.fields.is_empty() && self.fields.is_empty() {
            self.raw = true;
        } else if !shape.fields.is_empty() {
            self.raw = false;
        }

        for field in &shape.fields {
            self.child(&field.key).absorb(&field.shape);
        }
    }

    /// Descends a flatten path. `List` and `TypeCondition` do not consume a response key:
    /// list elements sit at the same position, and a polymorphic position is one node whose
    /// fields are the union of every branch.
    fn at_path(&mut self, path: &[FlattenNodePathSegment]) -> &mut MergedNode {
        let mut node = self;
        for segment in path {
            if let FlattenNodePathSegment::Field(key) = segment {
                node = node.child(key);
            }
        }
        node
    }

    fn build(&self) -> ResponseShape {
        if self.raw && self.fields.is_empty() {
            return ResponseShape {
                fields: Vec::new(),
                raw: true,
                inert: false,
            };
        }

        let fields: Vec<ResponseShapeField> = self
            .fields
            .iter()
            .map(|(key, child)| ResponseShapeField {
                key: key.clone(),
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

    fn node_at(&self, path: &[FlattenNodePathSegment]) -> Option<&MergedNode> {
        let mut node = self;
        for segment in path {
            if let FlattenNodePathSegment::Field(key) = segment {
                node = &node.fields.iter().find(|(k, _)| k == key)?.1;
            }
        }
        Some(node)
    }
}

/// Rewrites every fetch's `response_shape` to the merged tree's node for its position, so
/// responses landing in the same place share one slot layout.
/// Returns the merged shape of the whole response tree, and points every fetch at the node
/// for its own position.
pub fn unify_response_shapes(
    node: &mut PlanNode,
    operation: &OperationDefinition,
) -> ResponseShape {
    let mut root = MergedNode::default();
    // Client keys first, so their slots are stable across plans for the same operation;
    // planner-injected keys (`__typename`, key fields, aliases) append after them.
    root.absorb_selections(&operation.selection_set);
    accumulate(node, &FlattenNodePath::default(), &mut root);
    assign(node, &FlattenNodePath::default(), &root);

    // Slots exist only once every fetch has its final shape, so paths are compiled last.
    let root_shape = root.build();
    compile(node, &root_shape);
    root_shape
}

/// Resolves every response key a fetch addresses — its flatten path, `requires` and
/// rewrites — into slots, so nothing has to be looked up by name at request time.
fn compile(node: &mut PlanNode, root_shape: &ResponseShape) {
    match node {
        PlanNode::Fetch(fetch) => {
            let shape = &fetch.response_shape;
            fetch.compiled.entities_slot = shape.slot_of(ENTITIES_KEY);
            if let Some(rewrites) = &fetch.output_rewrites {
                fetch.compiled.output_rewrites = compile_rewrites(rewrites, shape);
            }
        }
        PlanNode::Subscription(subscription) => {
            let fetch = &mut subscription.primary;
            let shape = &fetch.response_shape;
            fetch.compiled.entities_slot = shape.slot_of(ENTITIES_KEY);
            if let Some(rewrites) = &fetch.output_rewrites {
                fetch.compiled.output_rewrites = compile_rewrites(rewrites, shape);
            }
        }
        PlanNode::BatchFetch(fetch) => {
            let batch_shape = fetch.response_shape.clone();
            for alias in &mut fetch.entity_batch.aliases {
                let alias_slot = batch_shape.slot_of(&alias.alias);
                alias.compiled.alias_slot = alias_slot;
                alias.compiled.merge_slot_paths = alias
                    .merge_paths
                    .iter()
                    .map(|path| compile_flatten_path(path, root_shape))
                    .collect();

                // `requires` and the rewrites read the position the entities merge into.
                let Some(target) = alias
                    .merge_paths
                    .first()
                    .and_then(|path| shape_at(root_shape, path))
                else {
                    continue;
                };
                alias.compiled.requires = compile_requires(&alias.requires, target);
                if let Some(rewrites) = &alias.input_rewrites {
                    alias.compiled.input_rewrites = compile_rewrites(rewrites, target);
                }
                if let Some(rewrites) = &alias.output_rewrites {
                    alias.compiled.output_rewrites = compile_rewrites(rewrites, target);
                }
            }
        }
        PlanNode::Flatten(flatten) => {
            flatten.slot_path = compile_flatten_path(&flatten.path, root_shape);
            if let PlanNode::Fetch(fetch) = flatten.node.as_mut() {
                fetch.compiled.entities_slot = fetch.response_shape.slot_of(ENTITIES_KEY);

                if let Some(target) = shape_at(root_shape, &flatten.path) {
                    if let Some(requires) = &fetch.requires {
                        fetch.compiled.requires = compile_requires(requires, target);
                    }
                    if let Some(rewrites) = &fetch.input_rewrites {
                        fetch.compiled.input_rewrites = compile_rewrites(rewrites, target);
                    }
                    if let Some(rewrites) = &fetch.output_rewrites {
                        fetch.compiled.output_rewrites = compile_rewrites(rewrites, target);
                    }
                }
            } else {
                compile(flatten.node.as_mut(), root_shape);
            }
        }
        PlanNode::Sequence(sequence) => {
            for child in &mut sequence.nodes {
                compile(child, root_shape);
            }
        }
        PlanNode::Parallel(parallel) => {
            for child in &mut parallel.nodes {
                compile(child, root_shape);
            }
        }
        PlanNode::Condition(condition) => {
            for branch in [&mut condition.if_clause, &mut condition.else_clause]
                .into_iter()
                .flatten()
            {
                compile(branch, root_shape);
            }
        }
        PlanNode::Defer(_) => {}
    }
}

/// The shape at a flatten path. List and type-condition segments do not consume a key.
fn shape_at<'a>(shape: &'a ResponseShape, path: &FlattenNodePath) -> Option<&'a ResponseShape> {
    let mut node = shape;
    for segment in path.as_slice() {
        if let FlattenNodePathSegment::Field(key) = segment {
            node = node.child(node.slot_of(key)?)?;
        }
    }
    Some(node)
}

/// The shape of one position, from the selections made at it. Same rule the merged tree
/// uses, so the slots line up with the data the executor builds.
pub fn response_shape_for_selections(selections: &SelectionSet) -> ResponseShape {
    let mut node = MergedNode::default();
    node.absorb_selections(selections);
    node.build()
}

/// The response shape for an operation the planner produced no fetches for — an
/// introspection-only query answers entirely from the router's own data.
pub fn response_shape_for_operation(operation: &OperationDefinition) -> ResponseShape {
    let mut root = MergedNode::default();
    root.absorb_selections(&operation.selection_set);
    root.build()
}

fn accumulate(node: &PlanNode, path: &FlattenNodePath, root: &mut MergedNode) {
    match node {
        PlanNode::Fetch(fetch) => {
            let node = root.at_path(path.as_slice());
            node.absorb(&fetch.response_shape);
            if let Some(rewrites) = &fetch.output_rewrites {
                node.absorb_rewrite_targets(rewrites);
            }
        }
        PlanNode::Subscription(subscription) => {
            root.at_path(path.as_slice())
                .absorb(&subscription.primary.response_shape);
        }
        PlanNode::BatchFetch(fetch) => {
            for alias in &fetch.entity_batch.aliases {
                let Some(alias_shape) = shape_for_key(&fetch.response_shape, &alias.alias) else {
                    continue;
                };
                for merge_path in &alias.merge_paths {
                    let node = root.at_path(merge_path.as_slice());
                    node.absorb(alias_shape);
                    node.absorb_requires(&alias.requires);
                    for rewrites in [&alias.input_rewrites, &alias.output_rewrites]
                        .into_iter()
                        .flatten()
                    {
                        node.absorb_rewrite_targets(rewrites);
                    }
                }
            }
        }
        PlanNode::Flatten(flatten) => {
            // An entity fetch answers `{"_entities": [...]}`; the elements land at the
            // flatten path, so that is the shape that belongs to the merged tree.
            let nested = concat_path(path, &flatten.path);
            match flatten.node.as_ref() {
                PlanNode::Fetch(fetch) => {
                    let shape = shape_for_key(&fetch.response_shape, ENTITIES_KEY)
                        .unwrap_or(&fetch.response_shape);
                    let node = root.at_path(nested.as_slice());
                    node.absorb(shape);
                    // `requires` reads from the position the entities merge into, not from
                    // the fetch's own output.
                    if let Some(requires) = &fetch.requires {
                        node.absorb_requires(requires);
                    }
                    for rewrites in [&fetch.input_rewrites, &fetch.output_rewrites]
                        .into_iter()
                        .flatten()
                    {
                        node.absorb_rewrite_targets(rewrites);
                    }
                }
                other => accumulate(other, &nested, root),
            }
        }
        PlanNode::Sequence(sequence) => {
            for child in &sequence.nodes {
                accumulate(child, path, root);
            }
        }
        PlanNode::Parallel(parallel) => {
            for child in &parallel.nodes {
                accumulate(child, path, root);
            }
        }
        PlanNode::Condition(condition) => {
            for branch in [&condition.if_clause, &condition.else_clause]
                .into_iter()
                .flatten()
            {
                accumulate(branch, path, root);
            }
        }
        PlanNode::Defer(_) => {}
    }
}

fn assign(node: &mut PlanNode, path: &FlattenNodePath, root: &MergedNode) {
    match node {
        PlanNode::Fetch(fetch) => {
            if let Some(merged) = root.node_at(path.as_slice()) {
                fetch.response_shape = merged.build();
            }
        }
        PlanNode::Subscription(subscription) => {
            if let Some(merged) = root.node_at(path.as_slice()) {
                subscription.primary.response_shape = merged.build();
            }
        }
        PlanNode::BatchFetch(fetch) => {
            let aliases = fetch.entity_batch.aliases.clone();
            let mut fields = Vec::with_capacity(aliases.len());
            for alias in &aliases {
                let Some(merge_path) = alias.merge_paths.first() else {
                    continue;
                };
                let Some(merged) = root.node_at(merge_path.as_slice()) else {
                    continue;
                };
                fields.push(ResponseShapeField {
                    key: alias.alias.clone(),
                    // The alias *is* the `_entities` field (`_e0: _entities(...)`), so its
                    // value is the entity array itself — no `_entities` level to wrap. Only a
                    // single entity fetch answers `{"_entities": [...]}`.
                    shape: merged.build(),
                });
            }
            let inert = fields.iter().all(|field| field.shape.inert);
            fetch.response_shape = ResponseShape {
                fields,
                raw: false,
                inert,
            };
        }
        PlanNode::Flatten(flatten) => {
            let nested = concat_path(path, &flatten.path);
            match flatten.node.as_mut() {
                PlanNode::Fetch(fetch) => {
                    if let Some(merged) = root.node_at(nested.as_slice()) {
                        // Only an entity call answers `{"_entities": [...]}`. A fetch under a
                        // flatten path with no `requires` is a subgraph re-entry: it answers
                        // the position's fields directly and is merged straight into it. The
                        // executor splits on the same condition.
                        fetch.response_shape = if fetch.requires.is_some() {
                            wrap_entities(merged.build())
                        } else {
                            merged.build()
                        };
                    }
                }
                other => assign(other, &nested, root),
            }
        }
        PlanNode::Sequence(sequence) => {
            for child in &mut sequence.nodes {
                assign(child, path, root);
            }
        }
        PlanNode::Parallel(parallel) => {
            for child in &mut parallel.nodes {
                assign(child, path, root);
            }
        }
        PlanNode::Condition(condition) => {
            for branch in [&mut condition.if_clause, &mut condition.else_clause]
                .into_iter()
                .flatten()
            {
                assign(branch, path, root);
            }
        }
        PlanNode::Defer(_) => {}
    }
}

fn wrap_entities(shape: ResponseShape) -> ResponseShape {
    let inert = shape.inert;
    ResponseShape {
        fields: vec![ResponseShapeField {
            key: ENTITIES_KEY.to_string(),
            shape,
        }],
        raw: false,
        inert,
    }
}

fn shape_for_key<'a>(shape: &'a ResponseShape, key: &str) -> Option<&'a ResponseShape> {
    shape
        .fields
        .iter()
        .find(|field| field.key == key)
        .map(|field| &field.shape)
}

fn concat_path(base: &FlattenNodePath, suffix: &FlattenNodePath) -> FlattenNodePath {
    if base.as_slice().is_empty() {
        return suffix.clone();
    }
    let mut segments = base.as_slice().to_vec();
    segments.extend_from_slice(suffix.as_slice());
    FlattenNodePath::from_segments(segments)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        graph::PlannerOverrideContext,
        planner::Planner,
        utils::{
            cancellation::CancellationToken,
            parsing::{parse_operation, parse_schema},
        },
    };

    fn plan(operation: &str) -> PlanNode {
        let sdl = std::fs::read_to_string("fixture/supergraph.graphql").expect("supergraph");
        let schema = parse_schema(&sdl);
        let planner = Planner::new_from_supergraph(&schema, Default::default()).expect("planner");
        let parsed = parse_operation(operation);
        let normalized =
            crate::ast::normalization::normalize_operation(&planner.supergraph, &parsed, None)
                .expect("normalized");
        planner
            .plan_from_normalized_operation(
                normalized.executable_operation(),
                PlannerOverrideContext::default(),
                &CancellationToken::new(),
            )
            .expect("plan")
            .node
            .expect("node")
    }

    /// Collects (position, field order) for every fetch, so two fetches landing in the same
    /// place can be compared.
    fn shapes_at(node: &PlanNode, path: Vec<String>, out: &mut Vec<(Vec<String>, Vec<String>)>) {
        let keys = |shape: &ResponseShape| -> Vec<String> {
            shape.fields.iter().map(|f| f.key.clone()).collect()
        };
        match node {
            PlanNode::Fetch(fetch) => {
                let shape = shape_for_key(&fetch.response_shape, ENTITIES_KEY)
                    .unwrap_or(&fetch.response_shape);
                out.push((path, keys(shape)));
            }
            PlanNode::Flatten(flatten) => {
                let mut nested = path;
                for segment in flatten.path.as_slice() {
                    if let FlattenNodePathSegment::Field(key) = segment {
                        nested.push(key.clone());
                    }
                }
                shapes_at(&flatten.node, nested, out);
            }
            PlanNode::Sequence(seq) => {
                for child in &seq.nodes {
                    shapes_at(child, path.clone(), out);
                }
            }
            PlanNode::Parallel(par) => {
                for child in &par.nodes {
                    shapes_at(child, path.clone(), out);
                }
            }
            _ => {}
        }
    }

    #[test]
    fn fetches_landing_in_the_same_place_share_one_field_order() {
        // `allProducts` is served by PRODUCTS, its reviews by REVIEWS, and the review
        // authors by USERS, so several fetches write into the same positions. They must
        // agree on slot layout or a positional merge would put values under the wrong
        // fields.
        let node = plan(
            r#"{ allProducts { id sku name reviewsCount reviews { id body } } }"#,
        );
        let mut shapes = Vec::new();
        shapes_at(&node, Vec::new(), &mut shapes);

        assert!(shapes.len() > 1, "expected a multi-fetch plan: {shapes:?}");

        for (position, keys) in &shapes {
            for (other_position, other_keys) in &shapes {
                if position == other_position {
                    assert_eq!(
                        keys, other_keys,
                        "two fetches at {position:?} disagree on field order: {keys:?} vs {other_keys:?}"
                    );
                }
            }
        }
    }

    /// Collects every fetch's response shape, tagged with how the executor will read it.
    fn fetch_shapes(node: &PlanNode, out: &mut Vec<(&'static str, ResponseShape)>) {
        match node {
            PlanNode::BatchFetch(fetch) => out.push(("batch", fetch.response_shape.clone())),
            PlanNode::Flatten(flatten) => match flatten.node.as_ref() {
                PlanNode::Fetch(fetch) => out.push((
                    if fetch.requires.is_some() {
                        "entity"
                    } else {
                        "reentry"
                    },
                    fetch.response_shape.clone(),
                )),
                other => fetch_shapes(other, out),
            },
            PlanNode::Sequence(seq) => seq.nodes.iter().for_each(|n| fetch_shapes(n, out)),
            PlanNode::Parallel(par) => par.nodes.iter().for_each(|n| fetch_shapes(n, out)),
            _ => {}
        }
    }

    #[test]
    fn entity_and_batch_fetches_are_shaped_for_the_envelope_they_actually_answer() {
        // A single entity fetch answers `{"_entities": [...]}`, so its shape needs that level.
        // A batched one aliases the field itself (`_e0: _entities(...)`), so the alias's value
        // IS the entity array and an `_entities` level there would make every entity's keys
        // unknown — silently dropping the whole response.
        let sdl = std::fs::read_to_string("../../bench/supergraph.graphql").expect("supergraph");
        let schema = parse_schema(&sdl);
        let planner = Planner::new_from_supergraph(&schema, Default::default()).expect("planner");
        let source = std::fs::read_to_string("../../bench/operation.graphql").expect("operation");
        let parsed = parse_operation(&source);
        let normalized =
            crate::ast::normalization::normalize_operation(&planner.supergraph, &parsed, None)
                .expect("normalized");
        let node = planner
            .plan_from_normalized_operation(
                normalized.executable_operation(),
                PlannerOverrideContext::default(),
                &CancellationToken::new(),
            )
            .expect("plan")
            .node
            .expect("node");

        let mut shapes = Vec::new();
        fetch_shapes(&node, &mut shapes);
        assert!(
            shapes.iter().any(|(kind, _)| *kind == "batch"),
            "the bench operation should produce a batched entity fetch"
        );

        for (kind, shape) in &shapes {
            match *kind {
                "entity" => assert_eq!(
                    shape.fields.iter().map(|f| f.key.as_str()).collect::<Vec<_>>(),
                    [ENTITIES_KEY],
                    "a single entity fetch answers {{\"_entities\": [...]}}"
                ),
                // A subgraph re-entry answers the position's fields directly and is merged
                // straight into it, so an `_entities` level would drop the whole response.
                "reentry" => assert!(
                    !shape.fields.iter().any(|f| f.key == ENTITIES_KEY),
                    "a re-entry fetch must not be shaped as an `_entities` envelope"
                ),
                "batch" => {
                    for field in &shape.fields {
                        assert!(
                            !field
                                .shape
                                .fields
                                .iter()
                                .any(|inner| inner.key == ENTITIES_KEY),
                            "alias {} must be shaped as the entity itself, not wrapped in \
                             another `_entities`",
                            field.key
                        );
                    }
                }
                _ => unreachable!(),
            }
        }
    }

    #[test]
    fn a_position_carries_the_union_of_every_fetch_that_writes_there() {
        let node = plan(r#"{ allProducts { id sku name reviewsCount reviews { id body } } }"#);
        let mut shapes = Vec::new();
        shapes_at(&node, Vec::new(), &mut shapes);

        let product_keys = shapes
            .iter()
            .find(|(position, _)| position == &vec!["allProducts".to_string()])
            .map(|(_, keys)| keys.clone())
            .unwrap_or_else(|| panic!("no shape at allProducts: {shapes:?}"));

        // `sku`/`name` come from PRODUCTS while `reviewsCount`/`reviews` come from REVIEWS,
        // and both have to be present in the one layout used at this position.
        for expected in ["sku", "name", "reviewsCount", "reviews"] {
            assert!(
                product_keys.iter().any(|k| k == expected),
                "merged shape at allProducts is missing {expected}: {product_keys:?}"
            );
        }
    }
}
