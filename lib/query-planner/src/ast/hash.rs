use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

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
    fn ast_hash<H: Hasher>(&self, hasher: &mut H) {
        // Order does not matter for hashing
        // The order of arguments does not matter.
        // To achieve order-insensitivity, we get all keys, sort them, and then
        // hash them with their values in that order.
        let mut keys: Vec<_> = self.keys().collect();
        keys.sort_unstable();
        for key in keys {
            key.hash(hasher);
            // We can unwrap here because we are iterating over existing keys
            self.get_argument(key).unwrap().ast_hash(hasher);
        }
    }
}

impl ASTHash for Vec<VariableDefinition> {
    fn ast_hash<H: Hasher>(&self, hasher: &mut H) {
        // Order does not matter for hashing
        // The order of variables does not matter.
        // We do a similar thing as for ArgumentsMap,
        // we sort the variables by their names.
        let mut variables: Vec<_> = self.iter().collect();
        variables.sort_by_key(|f| &f.name);
        for var in variables {
            var.ast_hash(hasher);
        }
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
