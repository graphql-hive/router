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

pub fn ast_hash(query: &OperationDefinition) -> u64 {
    let mut hasher = DefaultHasher::new();
    query.ast_hash(&mut hasher);
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

impl ASTHash for SelectionSet {
    fn ast_hash(&self, hasher: &mut ASTHasher) {
        for item in &self.items {
            item.ast_hash(hasher);
        }
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

impl ASTHash for &FieldSelection {
    fn ast_hash(&self, hasher: &mut ASTHasher) {
        self.name.hash(hasher);
        self.selections.ast_hash(hasher);
        if let Some(args) = &self.arguments {
            args.ast_hash(hasher);
        }
    }
}

impl ASTHash for &InlineFragmentSelection {
    fn ast_hash(&self, hasher: &mut ASTHasher) {
        self.type_condition.hash(hasher);
        self.selections.ast_hash(hasher);
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
