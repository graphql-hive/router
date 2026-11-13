use std::collections::BTreeSet;

use crate::executor::operation_filter::PathSegment;
use crate::pipeline::trie::{PathIndex, Trie};
use crate::query_planner::ast::flat::{
    arena::{Arena, ListId},
    plan_document::PlanDocument,
    Kind,
};

/// The document with the nulled fields dropped, and the variables nothing uses anymore.
pub(crate) fn rebuild_nulled_operation(
    original: &PlanDocument,
    nulled_field_trie: &Trie,
) -> PlanDocument {
    // new nodes and lists go on top of a copy, the untouched ones are reused by id
    let mut arena = original.arena.clone();
    let selections = rebuild_list(
        &mut arena,
        original.selections,
        nulled_field_trie,
        PathIndex::root(),
    );

    let mut used_variables = BTreeSet::new();
    arena.variable_usages(selections, &mut used_variables);
    let variables = original.variables.as_ref().map(|defs| {
        defs.iter()
            .filter(|def| used_variables.contains(&def.name))
            .cloned()
            .collect()
    });

    PlanDocument {
        arena,
        kind: original.kind.clone(),
        name: original.name.clone(),
        variables,
        selections,
        fragments: vec![],
    }
}

fn rebuild_list(
    arena: &mut Arena,
    list: ListId,
    nulled_field_trie: &Trie,
    path_position: PathIndex,
) -> ListId {
    // nothing below is nulled, keep it all
    if !nulled_field_trie.has_children(path_position) {
        return list;
    }

    let mut kept = Vec::with_capacity(arena.items(list).len());
    for index in 0..arena.items(list).len() {
        let id = arena.items(list)[index];
        let node = *arena.node(id);
        let segment = match node.kind {
            Kind::Field => PathSegment::Field(arena.response_key(id)),
            Kind::InlineFragment => PathSegment::Fragment(arena.str(node.name)),
            // spreads are inlined during normalization, there are none here
            Kind::FragmentSpread => continue,
        };

        let Some((child_position, is_nulled)) =
            nulled_field_trie.find_segment_at_position(path_position, segment)
        else {
            kept.push(id);
            continue;
        };

        // only a field gets nulled, a fragment's flag means nothing
        if is_nulled && node.kind == Kind::Field {
            continue;
        }

        let children = rebuild_list(arena, node.children, nulled_field_trie, child_position);
        if arena.is_empty(children) && !arena.is_empty(node.children) {
            continue;
        }

        let mut copy = node;
        copy.children = children;
        kept.push(arena.add_node(copy));
    }

    arena.list_of(&kept)
}
