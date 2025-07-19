use std::collections::{HashSet, VecDeque};

use petgraph::{
    graph::NodeIndex,
    visit::{EdgeRef, NodeRef},
};
use tracing::{instrument, trace};

use crate::{
    ast::{merge_path::MergePath, selection_set::find_arguments_conflicts},
    planner::fetch::{
        error::FetchGraphError,
        fetch_graph::FetchGraph,
        fetch_step_data::{FetchStepData, FetchStepFlags, FetchStepKind},
        selections::FetchStepSelections,
        state::MultiTypeFetchStep,
    },
};

// Return true in case an alias was applied during the merge process.
#[instrument(level = "trace", skip_all)]
pub(crate) fn perform_fetch_step_merge(
    self_index: NodeIndex,
    other_index: NodeIndex,
    fetch_graph: &mut FetchGraph<MultiTypeFetchStep>,
    force_merge_inputs: bool,
) -> Result<(), FetchGraphError> {
    let (me, other) = fetch_graph.get_pair_of_steps_mut(self_index, other_index)?;

    trace!(
        "merging fetch steps [{}] + [{}]",
        self_index.index(),
        other_index.index(),
    );

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
        other
            .output
            .migrate_from_another(&other.input, &MergePath::default())?;

        other.output.wrap_with_condition(condition);
    } else if me.is_entity_call() && other.condition.is_some() {
        // We don't want to pass a condition
        // to a regular (non-entity call) fetch step,
        // because the condition is applied on an inline fragment.
        me.condition = other.condition.take();
    }

    let scoped_aliases = me.output.safe_migrate_from_another(
        &other.output,
        &other.response_path.slice_from(me.response_path.len()),
        (
            me.flags.contains(FetchStepFlags::USED_FOR_REQUIRES),
            other.flags.contains(FetchStepFlags::USED_FOR_REQUIRES),
        ),
    )?;

    if !scoped_aliases.is_empty() {
        trace!(
            "Total of {} alises applied during safe merge of selections",
            scoped_aliases.len()
        );
        // In cases where merging a step resulted in internal aliasing, keep a record of the aliases.
        me.internal_aliases_locations.extend(scoped_aliases);
    }

    if let Some(input_rewrites) = other.input_rewrites.take() {
        if !input_rewrites.is_empty() {
            for input_rewrite in input_rewrites {
                me.add_input_rewrite(input_rewrite);
            }
        }
    }

    if force_merge_inputs {
        me.input
            .migrate_from_another(&other.input, &MergePath::default())?;
    } else if me.input.selecting_same_types(&other.input) {
        // It's safe to not check if a condition was turned into an inline fragment,
        // because if a condition is present and "me" is a non-entity fetch step,
        // then the type_name values of the inputs are different.
        if me.response_path != other.response_path {
            return Err(FetchGraphError::MismatchedResponsePath);
        }

        me.input
            .migrate_from_another(&other.input, &MergePath::default())?;
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
    graph: &FetchGraph<MultiTypeFetchStep>,
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

impl FetchStepData<MultiTypeFetchStep> {
    pub fn can_merge(
        &self,
        self_index: NodeIndex,
        other_index: NodeIndex,
        other: &Self,
        fetch_graph: &FetchGraph<MultiTypeFetchStep>,
    ) -> bool {
        if self_index == other_index {
            return false;
        }

        if self.service_name != other.service_name {
            return false;
        }

        // We allow to merge root with entity calls by adding an inline fragment with the @include/@skip
        if self.is_entity_call() && other.is_entity_call() && self.condition != other.condition {
            return false;
        }

        // If both are entities, their response_paths should match,
        // as we can't merge entity calls resolving different entities
        if matches!(self.kind, FetchStepKind::Entity) && self.kind == other.kind {
            if !self.response_path.eq(&other.response_path) {
                return false;
            }
        } else {
            // otherwise we can merge
            if !other.response_path.starts_with(&self.response_path) {
                return false;
            }
        }

        if self.has_arguments_conflicts_with(other) {
            return false;
        }

        // if the `other` FetchStep has a single parent and it's `this` FetchStep
        if fetch_graph.parents_of(other_index).count() == 1
            && fetch_graph
                .parents_of(other_index)
                .all(|edge| edge.source() == self_index)
        {
            return true;
        }

        // if they do not share parents, they can't be merged
        if !fetch_graph.parents_of(self_index).all(|self_edge| {
            fetch_graph
                .parents_of(other_index)
                .any(|other_edge| other_edge.source() == self_edge.source())
        }) {
            return false;
        }

        true
    }

    pub fn has_arguments_conflicts_with(&self, other: &Self) -> bool {
        let input_conflicts = FetchStepSelections::<MultiTypeFetchStep>::iter_matching_types(
            &self.input,
            &other.input,
            |_, self_selections, other_selections| {
                find_arguments_conflicts(self_selections, other_selections)
            },
        );

        input_conflicts
            .iter()
            .any(|(_, conflicts)| !conflicts.is_empty())
    }
}
