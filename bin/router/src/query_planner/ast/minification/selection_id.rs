use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::query_planner::ast::flat::arena::{Arena, ListId};

#[derive(Hash, PartialEq, Eq, Copy, Clone)]
pub struct SelectionId(u64);

pub fn generate_selection_id(arena: &Arena, type_name: &str, list: ListId) -> SelectionId {
    let mut hasher = DefaultHasher::new();
    type_name.hash(&mut hasher);
    // order matters: a fragment shared by `{ a b }` and `{ b a }` makes the subgraph send both
    // in one order, and the client then needs them reordered
    hash_list(arena, list, &mut hasher);
    SelectionId(hasher.finish())
}

/// Strings and arguments are interned, so equal symbols and argument ids are equal values.
fn hash_list(arena: &Arena, list: ListId, hasher: &mut DefaultHasher) {
    let items = arena.items(list);
    // the length keeps `{ a { b } c }` and `{ a { b c } }` apart
    hasher.write_usize(items.len());
    for &id in items {
        let node = arena.node(id);
        hasher.write_u8(node.kind as u8);
        hasher.write_u32(node.name);
        hasher.write_u32(node.alias);
        hasher.write_u32(node.args);
        hasher.write_u32(node.skip_if);
        hasher.write_u32(node.include_if);
        hasher.write_u8(node.omit_from_response as u8);
        hash_list(arena, node.children, hasher);
    }
}
