use rustc_hash::{FxBuildHasher, FxHashMap, FxHashSet, FxHasher};
use std::cell::RefCell;
use std::hash::{BuildHasher, Hash, Hasher};

use crate::query_planner::ast::arguments::ArgumentsMap;
use crate::query_planner::ast::fragment::FragmentDefinition;
use crate::query_planner::ast::operation::{OperationDefinition, VariableDefinition};
use crate::query_planner::ast::selection_item::SelectionItem;
use crate::query_planner::ast::selection_set::{
    FieldSelection, InlineFragmentSelection, SelectionSet,
};
use crate::query_planner::ast::value::Value;
use crate::query_planner::state::supergraph_state::{self, OperationKind, TypeNode};

/// A trait for hashing AST nodes, with support for both order-dependent and order-independent hashing.
pub struct SemanticShapeHashContext<'a> {
    fragments: &'a [FragmentDefinition],
    fragment_indices_by_name: FxHashMap<String, usize>,
    visiting_fragment_names: RefCell<FxHashSet<String>>,
}

impl<'a> SemanticShapeHashContext<'a> {
    pub fn new(fragments: &'a [FragmentDefinition]) -> Self {
        let mut fragment_indices_by_name = FxHashMap::default();
        for (index, fragment) in fragments.iter().enumerate() {
            fragment_indices_by_name.insert(fragment.name.clone(), index);
        }

        Self {
            fragments,
            fragment_indices_by_name,
            visiting_fragment_names: RefCell::new(FxHashSet::default()),
        }
    }

    fn get_fragment_by_name(&self, name: &str) -> Option<&'a FragmentDefinition> {
        let fragment_index = *self.fragment_indices_by_name.get(name)?;
        self.fragments.get(fragment_index)
    }

    fn mark_visiting(&self, name: &str) -> bool {
        self.visiting_fragment_names
            .borrow_mut()
            .insert(name.to_owned())
    }

    fn unmark_visiting(&self, name: &str) {
        self.visiting_fragment_names.borrow_mut().remove(name);
    }
}

pub trait ASTHash {
    fn ast_hash<H: Hasher, const ORDER_INDEPENDENT: bool>(&self, hasher: &mut H);

    /// Order-independent hashing with fragment spreads inlined
    fn semantic_shape_hash<H: Hasher>(&self, hasher: &mut H, _ctx: &SemanticShapeHashContext<'_>) {
        self.ast_hash::<_, true>(hasher);
    }
}

/// Computes the collision-resistant identity hash of an operation.
///
/// This value keys the query plan cache, so a collision would let one operation execute the
/// plan built for a different operation.
///
/// We use BLAKE3-128bit for that hash.
pub fn ast_hash(query: &OperationDefinition) -> u128 {
    let mut hasher = Blake3Hasher::default();
    query.ast_hash::<_, false>(&mut hasher);
    hasher.finish_u128()
}

/// A [`Hasher`] backed by BLAKE3 that can yield a 128-bit digest.
#[derive(Default)]
pub struct Blake3Hasher(blake3::Hasher);

impl Blake3Hasher {
    /// Returns the low 128 bits of the BLAKE3 digest of everything written so far.
    pub fn finish_u128(&self) -> u128 {
        let hash = self.0.finalize();
        u128::from_le_bytes(hash.as_bytes()[..16].try_into().unwrap())
    }
}

impl Hasher for Blake3Hasher {
    fn finish(&self) -> u64 {
        let hash = self.0.finalize();
        u64::from_le_bytes(hash.as_bytes()[..8].try_into().unwrap())
    }

    fn write(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }
}
// In all ShapeHash implementations, we never include anything to do with
// the position of the element in the query, i.e., fields that involve
// `Pos`

impl ASTHash for &OperationKind {
    fn ast_hash<H: Hasher, const ORDER_INDEPENDENT: bool>(&self, hasher: &mut H) {
        match self {
            OperationKind::Query => "kind:query".hash(hasher),
            OperationKind::Mutation => "kind:mutation".hash(hasher),
            OperationKind::Subscription => "kind:subscription".hash(hasher),
        }
    }
}

impl ASTHash for OperationDefinition {
    fn ast_hash<H: Hasher, const ORDER_INDEPENDENT: bool>(&self, hasher: &mut H) {
        self.operation_kind
            .as_ref()
            .or(Some(&supergraph_state::OperationKind::Query))
            .ast_hash::<_, ORDER_INDEPENDENT>(hasher);

        self.selection_set.ast_hash::<_, ORDER_INDEPENDENT>(hasher);
        self.variable_definitions
            .ast_hash::<_, ORDER_INDEPENDENT>(hasher);
    }
}

impl<T: ASTHash> ASTHash for Option<T> {
    fn ast_hash<H: Hasher, const ORDER_INDEPENDENT: bool>(&self, hasher: &mut H) {
        match self {
            None => false.hash(hasher),
            Some(t) => {
                Some(true).hash(hasher);
                t.ast_hash::<_, ORDER_INDEPENDENT>(hasher);
            }
        }
    }
}

impl ASTHash for SelectionSet {
    fn ast_hash<H: Hasher, const ORDER_INDEPENDENT: bool>(&self, hasher: &mut H) {
        // Length-prefix every selection set so its boundaries are explicit. Without it a
        // selection moved into or out of a nested set produces the same byte stream, e.g.
        // `node { ... on User { id name } }` and `node { ... on User { id } name }` both
        // emit `User, id, name` and collide.
        hasher.write_usize(self.items.len());

        if ORDER_INDEPENDENT {
            let mut combined_hash: u64 = 0;
            let build_hasher = FxBuildHasher;

            // To achieve an order-independent hash, we hash each key-value pair
            // individually and then combine their hashes using XOR (^).
            // Since XOR is commutative, the final hash is not affected by the iteration order.
            for item in &self.items {
                let mut key_val_hasher = build_hasher.build_hasher();
                item.ast_hash::<_, ORDER_INDEPENDENT>(&mut key_val_hasher);
                combined_hash ^= key_val_hasher.finish();
            }

            hasher.write_u64(combined_hash);
        } else {
            for item in &self.items {
                item.ast_hash::<_, ORDER_INDEPENDENT>(hasher);
            }
        }
    }

    fn semantic_shape_hash<H: Hasher>(&self, hasher: &mut H, ctx: &SemanticShapeHashContext<'_>) {
        // Use xor + sum + count to avoid collisions like {a b a a} vs {a b c c}
        let mut xor = 0u64;
        let mut sum = 0u64;
        let mut count = 0u64;

        for item in &self.items {
            let mut item_hasher = FxHasher::default();
            item.semantic_shape_hash(&mut item_hasher, ctx);
            let value = item_hasher.finish();
            xor ^= value;
            sum = sum.wrapping_add(value);
            count = count.wrapping_add(1);
        }

        xor.hash(hasher);
        sum.hash(hasher);
        count.hash(hasher);
    }
}

impl ASTHash for SelectionItem {
    fn ast_hash<H: Hasher, const ORDER_INDEPENDENT: bool>(&self, hasher: &mut H) {
        // A numeric tag per variant so the kinds can't be confused: a fragment spread named
        // `X` must not hash like a field or inline fragment that starts with `X`.
        match self {
            SelectionItem::Field(field) => {
                hasher.write_u8(0);
                field.ast_hash::<_, ORDER_INDEPENDENT>(hasher);
            }
            SelectionItem::InlineFragment(frag) => {
                hasher.write_u8(1);
                frag.ast_hash::<_, ORDER_INDEPENDENT>(hasher);
            }
            SelectionItem::FragmentSpread(name) => {
                hasher.write_u8(2);
                name.hash(hasher);
            }
        }
    }

    fn semantic_shape_hash<H: Hasher>(&self, hasher: &mut H, ctx: &SemanticShapeHashContext<'_>) {
        match self {
            SelectionItem::Field(field) => field.semantic_shape_hash(hasher, ctx),
            SelectionItem::InlineFragment(inline) => inline.semantic_shape_hash(hasher, ctx),
            SelectionItem::FragmentSpread(name) => {
                if !ctx.mark_visiting(name) {
                    // Cycle detected - hash nothing (unique marker)
                    return;
                }
                if let Some(fragment) = ctx.get_fragment_by_name(name) {
                    fragment.selection_set.semantic_shape_hash(hasher, ctx);
                }
                ctx.unmark_visiting(name);
            }
        }
    }
}

impl ASTHash for &FieldSelection {
    fn ast_hash<H: Hasher, const ORDER_INDEPENDENT: bool>(&self, hasher: &mut H) {
        self.name.hash(hasher);
        self.alias.hash(hasher);
        self.selections.ast_hash::<_, ORDER_INDEPENDENT>(hasher);

        if let Some(args) = &self.arguments {
            args.ast_hash::<_, ORDER_INDEPENDENT>(hasher);
        }

        if let Some(var_name) = self.include_if.as_ref() {
            "@include".hash(hasher);
            var_name.hash(hasher);
        }
        if let Some(var_name) = self.skip_if.as_ref() {
            "@skip".hash(hasher);
            var_name.hash(hasher);
        }

        self.omit_from_response.hash(hasher);
    }

    fn semantic_shape_hash<H: Hasher>(&self, hasher: &mut H, ctx: &SemanticShapeHashContext<'_>) {
        self.name.hash(hasher);
        self.alias.hash(hasher);
        self.selections.semantic_shape_hash(hasher, ctx);

        if let Some(args) = &self.arguments {
            args.ast_hash::<_, true>(hasher);
        }

        if let Some(var_name) = self.include_if.as_ref() {
            "@include".hash(hasher);
            var_name.hash(hasher);
        }
        if let Some(var_name) = self.skip_if.as_ref() {
            "@skip".hash(hasher);
            var_name.hash(hasher);
        }

        self.omit_from_response.hash(hasher);
    }
}

impl ASTHash for &InlineFragmentSelection {
    fn ast_hash<H: Hasher, const ORDER_INDEPENDENT: bool>(&self, hasher: &mut H) {
        self.type_condition.hash(hasher);
        self.selections.ast_hash::<_, ORDER_INDEPENDENT>(hasher);
        if let Some(var_name) = self.include_if.as_ref() {
            "@include".hash(hasher);
            var_name.hash(hasher);
        }
        if let Some(var_name) = self.skip_if.as_ref() {
            "@skip".hash(hasher);
            var_name.hash(hasher);
        }
    }

    fn semantic_shape_hash<H: Hasher>(&self, hasher: &mut H, ctx: &SemanticShapeHashContext<'_>) {
        // Include type_condition (key for "... on Product" vs "... on User")
        self.type_condition.hash(hasher);
        self.selections.semantic_shape_hash(hasher, ctx);

        if let Some(var_name) = self.include_if.as_ref() {
            "@include".hash(hasher);
            var_name.hash(hasher);
        }
        if let Some(var_name) = self.skip_if.as_ref() {
            "@skip".hash(hasher);
            var_name.hash(hasher);
        }
    }
}

impl ASTHash for ArgumentsMap {
    fn ast_hash<H: Hasher, const ORDER_INDEPENDENT: bool>(&self, state: &mut H) {
        // `ArgumentsMap` wraps a `BTreeMap`, so iteration is already in canonical key order
        // and the hash is order-independent without a lossy XOR fold. A length prefix keeps
        // it injective (so two argument sets can't be made to cancel out).
        state.write_usize(self.len());
        for (key, value) in self.into_iter() {
            key.hash(state);
            value.ast_hash::<_, ORDER_INDEPENDENT>(state);
        }
    }
}

impl ASTHash for Vec<VariableDefinition> {
    fn ast_hash<H: Hasher, const ORDER_INDEPENDENT: bool>(&self, hasher: &mut H) {
        // Sort by the (operation-unique) variable name for a canonical, injective order.
        // Also, adds a length prefix to improve XOR folds.
        let mut variables: Vec<&VariableDefinition> = self.iter().collect();
        variables.sort_unstable_by(|left, right| left.name.cmp(&right.name));

        hasher.write_usize(variables.len());
        for variable in variables {
            variable.ast_hash::<_, ORDER_INDEPENDENT>(hasher);
        }
    }
}

impl ASTHash for VariableDefinition {
    fn ast_hash<H: Hasher, const ORDER_INDEPENDENT: bool>(&self, hasher: &mut H) {
        self.name.hash(hasher);
        self.variable_type.ast_hash::<_, ORDER_INDEPENDENT>(hasher);
        self.default_value.ast_hash::<_, ORDER_INDEPENDENT>(hasher);
    }
}

impl ASTHash for TypeNode {
    fn ast_hash<H: Hasher, const ORDER_INDEPENDENT: bool>(&self, hasher: &mut H) {
        // A numeric tag per variant, rather than the literal strings `list`/`non_null`,
        // so a scalar type actually named `list` or `non_null` can't collide with a
        // list/non-null wrapper.
        match self {
            TypeNode::Named(name) => {
                hasher.write_u8(0);
                name.hash(hasher);
            }
            TypeNode::List(inner) => {
                hasher.write_u8(1);
                inner.ast_hash::<_, ORDER_INDEPENDENT>(hasher);
            }
            TypeNode::NonNull(inner) => {
                hasher.write_u8(2);
                inner.ast_hash::<_, ORDER_INDEPENDENT>(hasher);
            }
        }
    }
}

impl ASTHash for Value {
    fn ast_hash<H: Hasher, const ORDER_INDEPENDENT: bool>(&self, hasher: &mut H) {
        // A distinct discriminant per variant, plus a length prefix for the collection
        // variants, keeps this injective
        match self {
            Value::Variable(value) => {
                hasher.write_u8(0);
                value.hash(hasher);
            }
            Value::Int(value) => {
                hasher.write_u8(1);
                value.hash(hasher);
            }
            Value::Float(value) => {
                hasher.write_u8(2);
                if value.is_nan() {
                    panic!("Attempted to hash a NaN value");
                }

                value.to_bits().hash(hasher);
            }
            Value::String(value) => {
                hasher.write_u8(3);
                value.hash(hasher);
            }
            Value::Boolean(value) => {
                hasher.write_u8(4);
                value.hash(hasher);
            }
            Value::Null => {
                hasher.write_u8(5);
            }
            Value::Enum(value) => {
                hasher.write_u8(6);
                value.hash(hasher);
            }
            Value::List(values) => {
                hasher.write_u8(7);
                hasher.write_usize(values.len());
                for value in values {
                    value.ast_hash::<_, ORDER_INDEPENDENT>(hasher);
                }
            }
            Value::Object(map) => {
                hasher.write_u8(8);
                hasher.write_usize(map.len());
                // `map` is a `BTreeMap`, so iteration is already in canonical key order.
                for (name, value) in map {
                    name.hash(hasher);
                    value.ast_hash::<_, ORDER_INDEPENDENT>(hasher);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query_planner::ast::arguments::ArgumentsMap;
    use crate::query_planner::ast::operation::{OperationDefinition, VariableDefinition};
    use crate::query_planner::ast::selection_item::SelectionItem;
    use crate::query_planner::ast::selection_set::{
        FieldSelection, InlineFragmentSelection, SelectionSet,
    };
    use crate::query_planner::ast::value::Value;
    use crate::query_planner::state::supergraph_state::{OperationKind, TypeNode};
    use std::collections::BTreeMap;

    fn create_test_operation() -> OperationDefinition {
        let mut arguments = ArgumentsMap::new();
        arguments.add_argument("limit".to_string(), Value::Int(10));
        arguments.add_argument("sort".to_string(), Value::Enum("ASC".to_string()));

        let mut nested_object = BTreeMap::new();
        nested_object.insert(
            "nestedKey".to_string(),
            Value::String("nestedValue".to_string()),
        );

        arguments.add_argument("obj".to_string(), Value::Object(nested_object));

        let field_selection = FieldSelection {
            name: "users".to_string(),
            alias: Some("all_users".to_string()),
            selections: SelectionSet {
                items: vec![
                    SelectionItem::Field(FieldSelection {
                        name: "id".to_string(),
                        alias: None,
                        selections: SelectionSet { items: vec![] },
                        arguments: None,
                        include_if: None,
                        skip_if: None,
                        omit_from_response: false,
                    }),
                    SelectionItem::Field(FieldSelection {
                        name: "name".to_string(),
                        alias: None,
                        selections: SelectionSet { items: vec![] },
                        arguments: None,
                        include_if: Some("includeName".to_string()),
                        skip_if: None,
                        omit_from_response: false,
                    }),
                ],
            },
            arguments: Some(arguments),
            include_if: None,
            skip_if: Some("skipUsers".to_string()),
            omit_from_response: false,
        };

        let selection_set = SelectionSet {
            items: vec![SelectionItem::Field(field_selection)],
        };

        let variable_definitions = vec![
            VariableDefinition {
                name: "skipUsers".to_string(),
                variable_type: TypeNode::NonNull(Box::new(TypeNode::Named("Boolean".to_string()))),
                default_value: Some(Value::Boolean(false)),
            },
            VariableDefinition {
                name: "includeName".to_string(),
                variable_type: TypeNode::Named("Boolean".to_string()),
                default_value: None,
            },
        ];

        OperationDefinition {
            operation_kind: Some(OperationKind::Query),
            selection_set,
            variable_definitions: Some(variable_definitions),
            name: Some("TestQuery".to_string()),
        }
    }

    #[test]
    fn test_ast_hash_is_deterministic() {
        let operation = create_test_operation();

        let hash1 = ast_hash(&operation);
        let hash2 = ast_hash(&operation);

        // Test that the hash is consistent within the same run
        assert_eq!(hash1, hash2, "AST hash should be consistent");

        // Snapshot test: compare against a known, pre-calculated hash.
        // If the hashing logic changes, this value will need to be updated.
        let expected_hash = 201794293020440094835527077450610197299_u128;
        assert_eq!(
            hash1, expected_hash,
            "AST hash does not match the snapshot value. If this change is intentional, update the snapshot."
        );
    }

    fn user_field(alias: &str, id: Value) -> SelectionItem {
        let mut arguments = ArgumentsMap::new();
        arguments.add_argument("id".to_string(), id);

        SelectionItem::Field(FieldSelection {
            name: "user".to_string(),
            alias: Some(alias.to_string()),
            selections: SelectionSet { items: vec![] },
            arguments: Some(arguments),
            include_if: None,
            skip_if: None,
            omit_from_response: false,
        })
    }

    fn two_users(a: Value, b: Value) -> OperationDefinition {
        OperationDefinition {
            name: None,
            operation_kind: Some(OperationKind::Query),
            selection_set: SelectionSet {
                items: vec![user_field("a", a), user_field("b", b)],
            },
            variable_definitions: Some(vec![VariableDefinition {
                name: "u1".to_string(),
                variable_type: TypeNode::Named("ID".to_string()),
                default_value: None,
            }]),
        }
    }

    /// `Value::Variable("u1")` and `Value::String("u1")` used to hash identically (no
    /// variant tag), so these two operations shared a query plan cache key while needing
    /// different plans. The injective `Value` encoding must now keep them apart.
    #[test]
    fn test_hash_distinguishes_variable_from_literal_argument() {
        let variable_first = two_users(
            Value::Variable("u1".to_string()),
            Value::String("u1".to_string()),
        );
        let literal_first = two_users(
            Value::String("u1".to_string()),
            Value::Variable("u1".to_string()),
        );

        assert_ne!(
            ast_hash(&variable_first),
            ast_hash(&literal_first),
            "variable and string arguments must not hash to the same plan cache key"
        );
    }

    /// `Variable`, `String` and `Enum` wrapping the same text must all hash differently.
    #[test]
    fn test_hash_distinguishes_value_variants() {
        let as_variable = two_users(Value::Variable("x".to_string()), Value::Null);
        let as_string = two_users(Value::String("x".to_string()), Value::Null);
        let as_enum = two_users(Value::Enum("x".to_string()), Value::Null);

        assert_ne!(ast_hash(&as_variable), ast_hash(&as_string));
        assert_ne!(ast_hash(&as_variable), ast_hash(&as_enum));
        assert_ne!(ast_hash(&as_string), ast_hash(&as_enum));
    }

    /// Operation name is excluded and variable declaration order is not significant, so an
    /// identical operation with a different name and reordered variables hashes the same.
    #[test]
    fn test_hash_ignores_name_and_variable_order() {
        let mut renamed = create_test_operation();
        renamed.name = Some("Renamed".to_string());
        renamed
            .variable_definitions
            .as_mut()
            .expect("test operation has variables")
            .reverse();

        assert_eq!(ast_hash(&create_test_operation()), ast_hash(&renamed));
    }

    /// These two operations differ after normalization (`name` for `User` only vs
    /// `name` for every `Node`) and must not share a plan.
    #[test]
    fn test_hash_distinguishes_inline_fragment_boundaries() {
        use crate::query_planner::ast::normalization::normalize_operation;
        use crate::query_planner::state::supergraph_state::SupergraphState;
        use crate::query_planner::utils::parsing::parse_schema;
        use graphql_tools::parser::parse_query;

        let schema = parse_schema(
            r#"
            interface Node { id: ID! name: String }
            type User implements Node { id: ID! name: String }
            type Post implements Node { id: ID! name: String }
            type Query { node: Node }
            "#,
        );
        let supergraph = SupergraphState::new(&schema);
        let normalize = |query: &str| {
            normalize_operation(
                &supergraph,
                &parse_query(query).expect("to parse").into_static(),
                None,
            )
            .expect("to normalize")
            .operation
        };

        let name_inside = normalize("{ node { ... on User { id name } } }");
        let name_outside = normalize("{ node { ... on User { id } name } }");

        assert_ne!(
            name_inside.to_string(),
            name_outside.to_string(),
            "normalization must keep the operations distinct"
        );
        assert_ne!(
            ast_hash(&name_inside),
            ast_hash(&name_outside),
            "{} and {} share a plan cache key",
            name_inside,
            name_outside
        );
    }

    /// Moving a selection into or out of a nested selection set must change the hash. Both
    /// `node { ... on User { id name } }` and `node { ... on User { id } name }` emit the
    /// same `User, id, name` field stream, so without a per-selection-set length prefix they
    /// collide on one plan cache key.
    #[test]
    fn test_hash_distinguishes_nested_from_sibling_selection() {
        fn field(name: &str) -> SelectionItem {
            SelectionItem::Field(FieldSelection {
                name: name.to_string(),
                alias: None,
                selections: SelectionSet { items: vec![] },
                arguments: None,
                include_if: None,
                skip_if: None,
                omit_from_response: false,
            })
        }

        fn node_query(
            user_selections: Vec<SelectionItem>,
            node_siblings: Vec<SelectionItem>,
        ) -> OperationDefinition {
            let fragment = SelectionItem::InlineFragment(InlineFragmentSelection {
                type_condition: "User".to_string(),
                selections: SelectionSet {
                    items: user_selections,
                },
                include_if: None,
                skip_if: None,
            });
            let mut node_items = vec![fragment];
            node_items.extend(node_siblings);

            OperationDefinition {
                name: None,
                operation_kind: Some(OperationKind::Query),
                selection_set: SelectionSet {
                    items: vec![SelectionItem::Field(FieldSelection {
                        name: "node".to_string(),
                        alias: None,
                        selections: SelectionSet { items: node_items },
                        arguments: None,
                        include_if: None,
                        skip_if: None,
                        omit_from_response: false,
                    })],
                },
                variable_definitions: None,
            }
        }

        // `name` inside the fragment vs. `name` as a sibling of the fragment.
        let nested = node_query(vec![field("id"), field("name")], vec![]);
        let sibling = node_query(vec![field("id")], vec![field("name")]);

        assert_ne!(ast_hash(&nested), ast_hash(&sibling));
    }

    /// A scalar type literally named `list` or `non_null` must not hash like a list /
    /// non-null wrapper, now that the wrappers carry a numeric tag instead of those strings.
    #[test]
    fn test_hash_distinguishes_scalar_type_named_like_a_wrapper() {
        fn op_with_var_type(ty: TypeNode) -> OperationDefinition {
            OperationDefinition {
                name: None,
                operation_kind: Some(OperationKind::Query),
                selection_set: SelectionSet { items: vec![] },
                variable_definitions: Some(vec![VariableDefinition {
                    name: "v".to_string(),
                    variable_type: ty,
                    default_value: None,
                }]),
            }
        }

        let named_list = op_with_var_type(TypeNode::Named("list".to_string()));
        let list_wrapper =
            op_with_var_type(TypeNode::List(Box::new(TypeNode::Named("Int".to_string()))));
        let named_non_null = op_with_var_type(TypeNode::Named("non_null".to_string()));
        let non_null_wrapper = op_with_var_type(TypeNode::NonNull(Box::new(TypeNode::Named(
            "Int".to_string(),
        ))));

        assert_ne!(ast_hash(&named_list), ast_hash(&list_wrapper));
        assert_ne!(ast_hash(&named_non_null), ast_hash(&non_null_wrapper));
    }

    /// The selection item kinds must not be confused: a fragment spread named `x` must not
    /// hash like a field named `x`.
    #[test]
    fn test_hash_distinguishes_selection_item_kinds() {
        fn op(items: Vec<SelectionItem>) -> OperationDefinition {
            OperationDefinition {
                name: None,
                operation_kind: Some(OperationKind::Query),
                selection_set: SelectionSet { items },
                variable_definitions: None,
            }
        }

        let as_field = op(vec![SelectionItem::Field(FieldSelection {
            name: "x".to_string(),
            alias: None,
            selections: SelectionSet { items: vec![] },
            arguments: None,
            include_if: None,
            skip_if: None,
            omit_from_response: false,
        })]);
        let as_spread = op(vec![SelectionItem::FragmentSpread("x".to_string())]);

        assert_ne!(ast_hash(&as_field), ast_hash(&as_spread));
    }

    #[test]
    fn test_order_independent_hashing_for_arguments() {
        let mut args1 = ArgumentsMap::new();
        args1.add_argument("a".to_string(), Value::Int(1));
        args1.add_argument("b".to_string(), Value::Int(2));

        let mut args2 = ArgumentsMap::new();
        args2.add_argument("b".to_string(), Value::Int(2));
        args2.add_argument("a".to_string(), Value::Int(1));

        let mut hasher1 = FxHasher::default();
        args1.ast_hash::<_, true>(&mut hasher1);

        let mut hasher2 = FxHasher::default();
        args2.ast_hash::<_, true>(&mut hasher2);

        assert_eq!(
            hasher1.finish(),
            hasher2.finish(),
            "ArgumentsMap hashing should be order-independent"
        );
    }

    #[test]
    fn test_order_independent_hashing_for_variables() {
        let vars1 = vec![
            VariableDefinition {
                name: "varA".to_string(),
                variable_type: TypeNode::Named("String".to_string()),
                default_value: None,
            },
            VariableDefinition {
                name: "varB".to_string(),
                variable_type: TypeNode::Named("Int".to_string()),
                default_value: Some(Value::Int(0)),
            },
        ];

        let vars2 = vec![
            VariableDefinition {
                name: "varB".to_string(),
                variable_type: TypeNode::Named("Int".to_string()),
                default_value: Some(Value::Int(0)),
            },
            VariableDefinition {
                name: "varA".to_string(),
                variable_type: TypeNode::Named("String".to_string()),
                default_value: None,
            },
        ];

        let mut hasher1 = FxHasher::default();
        vars1.ast_hash::<_, true>(&mut hasher1);

        let mut hasher2 = FxHasher::default();
        vars2.ast_hash::<_, true>(&mut hasher2);

        assert_eq!(
            hasher1.finish(),
            hasher2.finish(),
            "VariableDefinition vector hashing should be order-independent"
        );
    }
}
