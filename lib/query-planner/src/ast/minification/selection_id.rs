use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::ast::hash::ASTHash;
use crate::ast::selection_set::SelectionSet;

#[derive(Hash, PartialEq, Eq, Copy, Clone)]
pub struct SelectionId(u64);

pub fn generate_selection_id(type_name: &str, selection_set: &SelectionSet) -> SelectionId {
    let mut hasher = DefaultHasher::new();
    type_name.hash(&mut hasher);
    selection_set.ast_hash::<_, true>(&mut hasher);
    SelectionId(hasher.finish())
}
