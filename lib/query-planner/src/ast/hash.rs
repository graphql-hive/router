use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::ast::arguments::ArgumentsMap;
use crate::ast::operation::OperationDefinition;
use crate::ast::selection_item::SelectionItem;
use crate::ast::selection_set::{FieldSelection, InlineFragmentSelection, SelectionSet};
use crate::ast::value::Value;
use crate::state::supergraph_state::{self, OperationKind};

type ASTHasher = DefaultHasher;

/// Order-dependent hashing
pub trait ASTHash {
    fn ast_hash(&self, hasher: &mut ASTHasher);
}

/// Order-independent hashing
pub trait ShapeHash {
    fn shape_hash(&self, hasher: &mut ASTHasher);
}

pub fn ast_hash(query: &OperationDefinition) -> u64 {
    let mut hasher = DefaultHasher::new();
    query.ast_hash(&mut hasher);
    hasher.finish()
}

pub fn shape_hash(query: &OperationDefinition) -> u64 {
    let mut hasher = DefaultHasher::new();
    query.shape_hash(&mut hasher);
    hasher.finish()
}

// In all ShapeHash implementations, we never include anything to do with
// the position of the element in the query, i.e., fields that involve
// `Pos`

impl ASTHash for &OperationKind {
    fn ast_hash(&self, hasher: &mut ASTHasher) {
        match self {
            OperationKind::Query => "Query".hash(hasher),
            OperationKind::Mutation => "Mutation".hash(hasher),
            OperationKind::Subscription => "Subscription".hash(hasher),
        }
    }
}

impl ShapeHash for &OperationKind {
    fn shape_hash(&self, hasher: &mut ASTHasher) {
        match self {
            OperationKind::Query => "Query".hash(hasher),
            OperationKind::Mutation => "Mutation".hash(hasher),
            OperationKind::Subscription => "Subscription".hash(hasher),
        }
    }
}

impl ASTHash for OperationDefinition {
    fn ast_hash(&self, hasher: &mut ASTHasher) {
        self.operation_kind
            .as_ref()
            .or(Some(&supergraph_state::OperationKind::Query))
            .ast_hash(hasher);

        self.selection_set.ast_hash(hasher);
        // TODO:
        // self.variable_definitions
        //     .or(Default::default())
        //     .shape_hash();
    }
}

impl ShapeHash for OperationDefinition {
    fn shape_hash(&self, hasher: &mut ASTHasher) {
        self.operation_kind
            .as_ref()
            .or(Some(&supergraph_state::OperationKind::Query))
            .shape_hash(hasher);

        self.selection_set.shape_hash(hasher);
        // TODO:
        // self.variable_definitions
        //     .or(Default::default())
        //     .shape_hash();
    }
}

impl<T: ASTHash> ASTHash for Option<T> {
    fn ast_hash(&self, hasher: &mut ASTHasher) {
        match self {
            None => false.hash(hasher),
            Some(t) => {
                Some(true).hash(hasher);
                t.ast_hash(hasher);
            }
        }
    }
}

impl<T: ShapeHash> ShapeHash for Option<T> {
    fn shape_hash(&self, hasher: &mut ASTHasher) {
        match self {
            None => false.hash(hasher),
            Some(t) => {
                Some(true).hash(hasher);
                t.shape_hash(hasher);
            }
        }
    }
}

impl ASTHash for SelectionSet {
    fn ast_hash(&self, hasher: &mut ASTHasher) {
        for item in &self.items {
            item.ast_hash(hasher);
        }
    }
}

impl ShapeHash for SelectionSet {
    fn shape_hash(&self, hasher: &mut ASTHasher) {
        // The order of selections does not matter
        let mut hashes: Vec<_> = self
            .items
            .iter()
            .map(|item| {
                let mut item_hasher = DefaultHasher::new();
                item.shape_hash(&mut item_hasher);
                item_hasher.finish()
            })
            .collect();
        hashes.sort_unstable();
        hashes.hash(hasher);
    }
}

impl ASTHash for SelectionItem {
    fn ast_hash(&self, hasher: &mut ASTHasher) {
        match self {
            SelectionItem::Field(field) => field.ast_hash(hasher),
            SelectionItem::InlineFragment(frag) => frag.ast_hash(hasher),
        }
    }
}

impl ShapeHash for SelectionItem {
    fn shape_hash(&self, hasher: &mut ASTHasher) {
        match self {
            SelectionItem::Field(field) => field.shape_hash(hasher),
            SelectionItem::InlineFragment(frag) => frag.shape_hash(hasher),
        }
    }
}

impl ASTHash for &FieldSelection {
    fn ast_hash(&self, hasher: &mut ASTHasher) {
        self.name.hash(hasher);
        self.selections.ast_hash(hasher);
        if let Some(args) = &self.arguments {
            args.ast_hash(hasher);
        }
    }
}

impl ShapeHash for &FieldSelection {
    fn shape_hash(&self, hasher: &mut ASTHasher) {
        self.name.hash(hasher);
        self.selections.shape_hash(hasher);
        if let Some(args) = &self.arguments {
            args.shape_hash(hasher);
        }
    }
}

impl ASTHash for &InlineFragmentSelection {
    fn ast_hash(&self, hasher: &mut ASTHasher) {
        self.type_condition.hash(hasher);
        self.selections.ast_hash(hasher);
    }
}

impl ShapeHash for &InlineFragmentSelection {
    fn shape_hash(&self, hasher: &mut ASTHasher) {
        self.type_condition.hash(hasher);
        self.selections.shape_hash(hasher);
    }
}

impl ASTHash for ArgumentsMap {
    fn ast_hash(&self, hasher: &mut ASTHasher) {
        for (name, value) in self {
            name.hash(hasher);
            value.ast_hash(hasher);
        }
    }
}

impl ShapeHash for ArgumentsMap {
    fn shape_hash(&self, hasher: &mut ASTHasher) {
        // The order of arguments does not matter.
        // To achieve order-insensitivity, we get all keys, sort them, and then
        // hash them with their values in that order.
        let mut keys: Vec<_> = self.keys().collect();
        keys.sort_unstable();
        for key in keys {
            key.hash(hasher);
            // We can unwrap here because we are iterating over existing keys
            self.get_argument(key).unwrap().shape_hash(hasher);
        }
    }
}

impl ASTHash for Value {
    fn ast_hash(&self, hasher: &mut ASTHasher) {
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

impl ShapeHash for Value {
    fn shape_hash(&self, hasher: &mut ASTHasher) {
        match self {
            Value::List(values) => {
                // The order of values in a list does not matter for the query shape
                let mut hashes: Vec<_> = values
                    .iter()
                    .map(|value| {
                        let mut value_hasher = DefaultHasher::new();
                        value.shape_hash(&mut value_hasher);
                        value_hasher.finish()
                    })
                    .collect();
                hashes.sort_unstable();
                hashes.hash(hasher);
            }
            Value::Object(map) => {
                // The order of fields in an object does not matter for the query shape
                let mut keys: Vec<_> = map.keys().collect();
                keys.sort_unstable();
                for key in keys {
                    key.hash(hasher);
                    // We can unwrap here because we are iterating over existing keys
                    map.get(key).unwrap().shape_hash(hasher);
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
