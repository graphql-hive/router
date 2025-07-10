use std::collections::{HashSet, VecDeque};

use petgraph::{
    graph::NodeIndex,
    visit::{EdgeRef, NodeRef},
};
use tracing::{instrument, trace};

use crate::{
    ast::{
        merge_path::Condition, selection_item::SelectionItem,
        selection_set::InlineFragmentSelection,
    },
    planner::fetch::{error::FetchGraphError, fetch_graph::FetchGraph},
};

// Return true in case an alias was applied during the merge process.
#[instrument(level = "trace", skip_all)]
pub(crate) fn perform_fetch_step_merge(
    self_index: NodeIndex,
    other_index: NodeIndex,
    fetch_graph: &mut FetchGraph,
) -> Result<(), FetchGraphError> {
    let (me, other) = fetch_graph.get_pair_of_steps_mut(self_index, other_index)?;

    trace!(
        "merging fetch steps [{}] + [{}]",
        self_index.index(),
        other_index.index(),
    );

    trace!("self: {}", me);
    trace!("other: {}", other);

    if let (true, Some(condition)) = (!me.is_entity_call(), other.condition.clone()) {
        // The "other" fetch step is an entity call,
        // that has both input and output, obviously.
        // We can safely merge them into the output of the "me" fetch step,
        // as it's a regular query (not an entity call).
        //
        // Input             -> Output
        // { id __typename } -> { price }
        //
        // becomes
        // { products { ... on Product @skip(if: $bool) { __typename id price } } }
        //
        // We can't do it when both are entity calls fetch steps.
        // TODO: Bring back
        // other
        //     .output_new
        //     .add_at_root(&other.input.type_name, other.input.selection_set);

        // let old_selection_set = other.output.selection_set.clone();
        // match condition {
        //     Condition::Include(var_name) => {
        //         other.output.selection_set.items =
        //             vec![SelectionItem::InlineFragment(InlineFragmentSelection {
        //                 type_condition: other.output.type_name.clone(),
        //                 selections: old_selection_set,
        //                 skip_if: None,
        //                 include_if: Some(var_name),
        //             })];
        //     }
        //     Condition::Skip(var_name) => {
        //         other.output.selection_set.items =
        //             vec![SelectionItem::InlineFragment(InlineFragmentSelection {
        //                 type_condition: other.output.type_name.clone(),
        //                 selections: old_selection_set,
        //                 skip_if: Some(var_name),
        //                 include_if: None,
        //             })];
        //     }
        // }
    } else if me.is_entity_call() && other.condition.is_some() {
        // We don't want to pass a condition
        // to a regular (non-entity call) fetch step,
        // because the condition is applied on an inline fragment.
        me.condition = other.condition.take();
    }

    let scoped_aliases = me.output_new.safe_add_from_another_at_path(
        &other.output_new,
        &other.response_path.slice_from(me.response_path.len()),
        (me.used_for_requires, other.used_for_requires),
    );

    // In cases where merging a step resulted in internal aliasing, keep a record of the aliases.
    me.internal_aliases_locations.extend(scoped_aliases);

    if let Some(input_rewrites) = other.input_rewrites.take() {
        if !input_rewrites.is_empty() {
            for input_rewrite in input_rewrites {
                me.add_input_rewrite(input_rewrite);
            }
        }
    }

    // It's safe to not check if a condition was turned into an inline fragment,
    // because if a condition is present and "me" is a non-entity fetch step,
    // then the type_name values of the inputs are different.
    if me.input.type_name == other.input.type_name {
        if me.response_path != other.response_path {
            return Err(FetchGraphError::MismatchedResponsePath);
        }

        me.input.add(&other.input);
    }

    let mut children_indexes: Vec<NodeIndex> = vec![];
    let mut parents_indexes: Vec<NodeIndex> = vec![];
    for edge_ref in fetch_graph.children_of(other_index) {
        children_indexes.push(edge_ref.target().id());
    }

    for edge_ref in fetch_graph.parents_of(other_index) {
        // We ignore self_index
        if edge_ref.source().id() != self_index {
            parents_indexes.push(edge_ref.source().id());
        }
    }

    // Replace parents:
    // 1. Add self -> child
    for child_index in children_indexes.iter() {
        fetch_graph.connect(self_index, *child_index);
    }
    // 2. Add parent -> self
    for parent_index in parents_indexes {
        fetch_graph.connect(parent_index, self_index);
    }
    // 3. Drop other -> child and parent -> other
    fetch_graph.remove_step(other_index);

    Ok(())
}

/// Checks if an ancestor node (`target_ancestor_index`) is reachable from a
/// child node (`child_index`) in a directed graph by following paths upwards
/// (traversing incoming edges), EXCLUDING any paths that start by traversing
/// the direct edge from the `target_ancestor_index` down to the `child_index`.
///
/// This is implemented as an iterative Breadth-First Search (BFS).
/// The search starts from all direct parents of `child_index` *except*
/// `target_ancestor_index`, and follows incoming edges from there.
pub fn is_reachable_via_alternative_upstream_path(
    graph: &FetchGraph,
    child_index: NodeIndex,
    target_ancestor_index: NodeIndex,
) -> Result<bool, FetchGraphError> {
    let mut queue: VecDeque<NodeIndex> = VecDeque::new();
    let mut visited: HashSet<NodeIndex> = HashSet::new();

    // Start BFS queue with all parents of `child_index` except `target_ancestor_index`
    for edge_ref in graph.parents_of(child_index) {
        let parent_index = edge_ref.source();

        if parent_index != target_ancestor_index {
            queue.push_back(parent_index);
            visited.insert(parent_index);
        }
    }

    if queue.is_empty() {
        return Ok(false);
    }

    // Perform BFS upwards (following incoming edges)
    while let Some(current_index) = queue.pop_front() {
        // If we reached the target ancestor indirectly
        if current_index == target_ancestor_index {
            return Ok(true);
        }

        // Explore further up the graph via the parents of the current node
        for edge_ref in graph.parents_of(current_index) {
            let parent_of_current_index = edge_ref.source();

            if visited.insert(parent_of_current_index) {
                queue.push_back(parent_of_current_index);
            }
        }
    }

    // no indirect path exists
    Ok(false)
}
