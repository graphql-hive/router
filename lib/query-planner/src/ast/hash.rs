use std::collections::hash_map::DefaultHasher;
use std::hash::{BuildHasher, Hash, Hasher, RandomState};

use crate::ast::arguments::ArgumentsMap;
use crate::ast::operation::{OperationDefinition, VariableDefinition};
use crate::ast::selection_item::SelectionItem;
use crate::ast::selection_set::{FieldSelection, InlineFragmentSelection, SelectionSet};
use crate::ast::value::Value;
use crate::state::supergraph_state::{self, OperationKind, TypeNode};

/// Order-dependent hashing
pub trait ASTHash {
    fn ast_hash<H: Hasher>(&self, hasher: &mut H);
}

pub fn ast_hash(query: &OperationDefinition) -> u64 {
    let mut hasher = DefaultHasher::new();
    query.ast_hash(&mut hasher);
    hasher.finish()
}
// In all ShapeHash implementations, we never include anything to do with
// the position of the element in the query, i.e., fields that involve
// `Pos`

impl ASTHash for &OperationKind {
    fn ast_hash<H: Hasher>(&self, hasher: &mut H) {
        match self {
            OperationKind::Query => "Query".hash(hasher),
            OperationKind::Mutation => "Mutation".hash(hasher),
            OperationKind::Subscription => "Subscription".hash(hasher),
        }
    }
}

impl ASTHash for OperationDefinition {
    fn ast_hash<H: Hasher>(&self, hasher: &mut H) {
        self.operation_kind
            .as_ref()
            .or(Some(&supergraph_state::OperationKind::Query))
            .ast_hash(hasher);

        self.selection_set.ast_hash(hasher);
        self.variable_definitions.ast_hash(hasher);
    }
}

impl<T: ASTHash> ASTHash for Option<T> {
    fn ast_hash<H: Hasher>(&self, hasher: &mut H) {
        match self {
            None => false.hash(hasher),
            Some(t) => {
                Some(true).hash(hasher);
                t.ast_hash(hasher);
            }
        }
    }
}

impl ASTHash for SelectionSet {
    fn ast_hash<H: Hasher>(&self, hasher: &mut H) {
        for item in &self.items {
            item.ast_hash(hasher);
        }
    }
}

impl ASTHash for SelectionItem {
    fn ast_hash<H: Hasher>(&self, hasher: &mut H) {
        match self {
            SelectionItem::Field(field) => field.ast_hash(hasher),
            SelectionItem::InlineFragment(frag) => frag.ast_hash(hasher),
            SelectionItem::FragmentSpread(name) => name.hash(hasher),
        }
    }
}

impl ASTHash for &FieldSelection {
    fn ast_hash<H: Hasher>(&self, hasher: &mut H) {
        self.name.hash(hasher);
        self.alias.hash(hasher);
        self.selections.ast_hash(hasher);

        if let Some(args) = &self.arguments {
            args.ast_hash(hasher);
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
    fn ast_hash<H: Hasher>(&self, hasher: &mut H) {
        self.type_condition.hash(hasher);
        self.selections.ast_hash(hasher);
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
    fn ast_hash<H: Hasher>(&self, state: &mut H) {
        let mut combined_hash: u64 = 0;
        let build_hasher = RandomState::new();

        // To achieve an order-independent hash, we hash each key-value pair
        // individually and then combine their hashes using XOR (^).
        // Since XOR is commutative, the final hash is not affected by the iteration order.
        for (key, value) in self.into_iter() {
            let mut key_val_hasher = build_hasher.build_hasher();
            key.hash(&mut key_val_hasher);
            value.ast_hash(&mut key_val_hasher);
            combined_hash ^= key_val_hasher.finish();
        }

        state.write_u64(combined_hash);
    }
}

impl ASTHash for Vec<VariableDefinition> {
    fn ast_hash<H: Hasher>(&self, hasher: &mut H) {
        let mut combined_hash: u64 = 0;
        let build_hasher = RandomState::new();
        // To achieve an order-independent hash, we hash each key-value pair
        // individually and then combine their hashes using XOR (^).
        // Since XOR is commutative, the final hash is not affected by the iteration order.
        for variable in self.into_iter() {
            let mut local_hasher = build_hasher.build_hasher();
            variable.ast_hash(&mut local_hasher);
            combined_hash ^= local_hasher.finish();
        }

        hasher.write_u64(combined_hash);
    }
}

impl ASTHash for VariableDefinition {
    fn ast_hash<H: Hasher>(&self, hasher: &mut H) {
        self.name.hash(hasher);
        self.variable_type.ast_hash(hasher);
        self.default_value.ast_hash(hasher);
    }
}

impl ASTHash for TypeNode {
    fn ast_hash<H: Hasher>(&self, hasher: &mut H) {
        match self {
            TypeNode::Named(name) => name.hash(hasher),
            TypeNode::List(inner) => {
                "list".hash(hasher);
                inner.ast_hash(hasher);
            }
            TypeNode::NonNull(inner) => {
                "non_null".hash(hasher);
                inner.ast_hash(hasher);
            }
        }
    }
}

impl ASTHash for Value {
    fn ast_hash<H: Hasher>(&self, hasher: &mut H) {
        match self {
            Value::List(values) => {
                for value in values {
                    value.ast_hash(hasher);
                }
            }
            Value::Object(map) => {
                for (name, value) in map {
                    name.hash(hasher);
                    value.ast_hash(hasher);
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
