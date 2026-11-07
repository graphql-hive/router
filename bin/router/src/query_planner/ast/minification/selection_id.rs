use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::query_planner::ast::hash::ASTHash;
use crate::query_planner::ast::selection_set::SelectionSet;

#[derive(Hash, PartialEq, Eq, Copy, Clone)]
pub struct SelectionId(u64);

pub fn generate_selection_id(type_name: &str, selection_set: &SelectionSet) -> SelectionId {
    let mut hasher = DefaultHasher::new();
    type_name.hash(&mut hasher);
    // order matters: a fragment shared by `{ a b }` and `{ b a }` makes the subgraph send both
    // in one order, and the client then needs them reordered
    selection_set.ast_hash::<_, false>(&mut hasher);
    SelectionId(hasher.finish())
}
