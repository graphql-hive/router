use rustc_hash::{FxBuildHasher, FxHasher};
use std::hash::{BuildHasher, Hash, Hasher};

use crate::ast::arguments::ArgumentsMap;
use crate::ast::operation::{OperationDefinition, VariableDefinition};
use crate::ast::selection_item::SelectionItem;
use crate::ast::selection_set::{FieldSelection, InlineFragmentSelection, SelectionSet};
use crate::ast::value::Value;
use crate::state::supergraph_state::{self, OperationKind, TypeNode};

/// Order-dependent hashing
pub trait ASTHash {
    fn ast_hash<H: Hasher>(&self, order_independent: bool, hasher: &mut H);
}

pub fn ast_hash(query: &OperationDefinition) -> u64 {
    let mut hasher = FxHasher::default();
    query.ast_hash(false, &mut hasher);
    hasher.finish()
}
// In all ShapeHash implementations, we never include anything to do with
// the position of the element in the query, i.e., fields that involve
// `Pos`

impl ASTHash for &OperationKind {
    fn ast_hash<H: Hasher>(&self, _order_independent: bool, hasher: &mut H) {
        match self {
            OperationKind::Query => "Query".hash(hasher),
            OperationKind::Mutation => "Mutation".hash(hasher),
            OperationKind::Subscription => "Subscription".hash(hasher),
        }
    }
}

impl ASTHash for OperationDefinition {
    fn ast_hash<H: Hasher>(&self, order_independent: bool, hasher: &mut H) {
        self.operation_kind
            .as_ref()
            .or(Some(&supergraph_state::OperationKind::Query))
            .ast_hash(order_independent, hasher);

        self.selection_set.ast_hash(order_independent, hasher);
        self.variable_definitions
            .ast_hash(order_independent, hasher);
    }
}

impl<T: ASTHash> ASTHash for Option<T> {
    fn ast_hash<H: Hasher>(&self, order_independent: bool, hasher: &mut H) {
        match self {
            None => false.hash(hasher),
            Some(t) => {
                Some(true).hash(hasher);
                t.ast_hash(order_independent, hasher);
            }
        }
    }
}

impl ASTHash for SelectionSet {
    fn ast_hash<H: Hasher>(&self, order_independent: bool, hasher: &mut H) {
        if order_independent {
            let mut combined_hash: u64 = 0;
            let build_hasher = FxBuildHasher;

            // To achieve an order-independent hash, we hash each key-value pair
            // individually and then combine their hashes using XOR (^).
            // Since XOR is commutative, the final hash is not affected by the iteration order.
            for item in &self.items {
                let mut key_val_hasher = build_hasher.build_hasher();
                item.ast_hash(true, &mut key_val_hasher);
                combined_hash ^= key_val_hasher.finish();
            }

            hasher.write_u64(combined_hash);
        } else {
            for item in &self.items {
                item.ast_hash(false, hasher);
            }
        }
    }
}

impl ASTHash for SelectionItem {
    fn ast_hash<H: Hasher>(&self, order_independent: bool, hasher: &mut H) {
        match self {
            SelectionItem::Field(field) => field.ast_hash(order_independent, hasher),
            SelectionItem::InlineFragment(frag) => frag.ast_hash(order_independent, hasher),
            SelectionItem::FragmentSpread(name) => name.hash(hasher),
        }
    }
}

impl ASTHash for &FieldSelection {
    fn ast_hash<H: Hasher>(&self, order_independent: bool, hasher: &mut H) {
        self.name.hash(hasher);
        self.alias.hash(hasher);
        self.selections.ast_hash(order_independent, hasher);

        if let Some(args) = &self.arguments {
            args.ast_hash(order_independent, hasher);
        }

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

impl ASTHash for &InlineFragmentSelection {
    fn ast_hash<H: Hasher>(&self, order_independent: bool, hasher: &mut H) {
        self.type_condition.hash(hasher);
        self.selections.ast_hash(order_independent, hasher);
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
    fn ast_hash<H: Hasher>(&self, order_independent: bool, state: &mut H) {
        let mut combined_hash: u64 = 0;
        let build_hasher = FxBuildHasher;

        // To achieve an order-independent hash, we hash each key-value pair
        // individually and then combine their hashes using XOR (^).
        // Since XOR is commutative, the final hash is not affected by the iteration order.
        for (key, value) in self.into_iter() {
            let mut key_val_hasher = build_hasher.build_hasher();
            key.hash(&mut key_val_hasher);
            value.ast_hash(order_independent, &mut key_val_hasher);
            combined_hash ^= key_val_hasher.finish();
        }

        state.write_u64(combined_hash);
    }
}

impl ASTHash for Vec<VariableDefinition> {
    fn ast_hash<H: Hasher>(&self, order_independent: bool, hasher: &mut H) {
        let mut combined_hash: u64 = 0;
        let build_hasher = FxBuildHasher;
        // To achieve an order-independent hash, we hash each key-value pair
        // individually and then combine their hashes using XOR (^).
        // Since XOR is commutative, the final hash is not affected by the iteration order.
        for variable in self.iter() {
            let mut local_hasher = build_hasher.build_hasher();
            variable.ast_hash(order_independent, &mut local_hasher);
            combined_hash ^= local_hasher.finish();
        }

        hasher.write_u64(combined_hash);
    }
}

impl ASTHash for VariableDefinition {
    fn ast_hash<H: Hasher>(&self, order_independent: bool, hasher: &mut H) {
        self.name.hash(hasher);
        self.variable_type.ast_hash(order_independent, hasher);
        self.default_value.ast_hash(order_independent, hasher);
    }
}

impl ASTHash for TypeNode {
    fn ast_hash<H: Hasher>(&self, order_independent: bool, hasher: &mut H) {
        match self {
            TypeNode::Named(name) => name.hash(hasher),
            TypeNode::List(inner) => {
                "list".hash(hasher);
                inner.ast_hash(order_independent, hasher);
            }
            TypeNode::NonNull(inner) => {
                "non_null".hash(hasher);
                inner.ast_hash(order_independent, hasher);
            }
        }
    }
}

impl ASTHash for Value {
    fn ast_hash<H: Hasher>(&self, order_independent: bool, hasher: &mut H) {
        match self {
            Value::List(values) => {
                for value in values {
                    value.ast_hash(order_independent, hasher);
                }
            }
            Value::Object(map) => {
                for (name, value) in map {
                    name.hash(hasher);
                    value.ast_hash(order_independent, hasher);
                }
            }
            Value::Null => {
                "null".hash(hasher);
            }
            Value::Int(value) => value.hash(hasher),
            Value::Float(value) => {
                if value.is_nan() {
                    panic!("Attempted to hash a NaN value");
                }

                value.to_bits().hash(hasher);
            }
            Value::Enum(value) => value.hash(hasher),
            Value::Boolean(value) => value.hash(hasher),
            Value::String(value) => value.hash(hasher),
            Value::Variable(value) => value.hash(hasher),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::arguments::ArgumentsMap;
    use crate::ast::operation::{OperationDefinition, VariableDefinition};
    use crate::ast::selection_item::SelectionItem;
    use crate::ast::selection_set::{FieldSelection, SelectionSet};
    use crate::ast::value::Value;
    use crate::state::supergraph_state::{OperationKind, TypeNode};
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
                    }),
                    SelectionItem::Field(FieldSelection {
                        name: "name".to_string(),
                        alias: None,
                        selections: SelectionSet { items: vec![] },
                        arguments: None,
                        include_if: Some("includeName".to_string()),
                        skip_if: None,
                    }),
                ],
            },
            arguments: Some(arguments),
            include_if: None,
            skip_if: Some("skipUsers".to_string()),
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
        let expected_hash = 8854078506550230644;
        assert_eq!(
            hash1, expected_hash,
            "AST hash does not match the snapshot value. If this change is intentional, update the snapshot."
        );
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
        args1.ast_hash(true, &mut hasher1);

        let mut hasher2 = FxHasher::default();
        args2.ast_hash(true, &mut hasher2);

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
        vars1.ast_hash(true, &mut hasher1);

        let mut hasher2 = FxHasher::default();
        vars2.ast_hash(true, &mut hasher2);

        assert_eq!(
            hasher1.finish(),
            hasher2.finish(),
            "VariableDefinition vector hashing should be order-independent"
        );
    }
}
