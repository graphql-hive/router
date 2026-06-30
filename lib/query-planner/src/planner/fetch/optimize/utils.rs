use std::collections::{HashSet, VecDeque};

use petgraph::{
    graph::NodeIndex,
    visit::{EdgeRef, NodeRef},
};
use tracing::{instrument, trace};

use crate::{
    ast::{
        merge_path::{MergePath, Segment},
        selection_set::find_arguments_conflicts,
    },
    planner::fetch::{
        error::FetchGraphError,
        fetch_graph::FetchGraph,
        fetch_step_data::{FetchStepData, FetchStepFlags, FetchStepKind},
        selections::FetchStepSelections,
        state::MultiTypeFetchStep,
    },
};

/// Handles the "target is non-entity, source has step-level condition" case.
/// When merging an entity fetch into a non-entity target, the condition must
/// stay attached to the source branch data, not to the whole target step.
fn merge_source_condition_into_non_entity_target<'a>(
    target: &FetchStepData<'a, MultiTypeFetchStep>,
    source: &mut FetchStepData<'a, MultiTypeFetchStep>,
) -> Result<bool, FetchGraphError> {
    let Some(condition) = source.condition.clone() else {
        return Ok(false);
    };

    if target.is_entity_call() {
        return Ok(false);
    }

    // The source fetch step is an entity call, so it has both input and output.
    // The target fetch step is a regular (non-entity) query fetch.
    //
    // We can safely migrate source.input into source.output before merging:
    //
    // Input             -> Output
    // { id __typename } -> { price }
    //
    // becomes
    // { products { ... on Product @skip(if: $bool) { __typename id price } } }
    //
    // We do this only for non-entity target merges. Entity-to-entity merges use
    // different path/type rules and are handled in a separate branch.
    source
        .output
        .migrate_from_another(&source.input, &MergePath::default())?;

    // Check if the condition is already enforced by the path
    let condition_redundant = matches!(
        source.response_path.last(),
        Some(Segment::TypeCondition(_, Some(c)) | Segment::Field(_, _, Some(c))) if c == &condition
    );

    if !condition_redundant {
        source.output.wrap_with_condition(condition);
    }

    Ok(true)
}

// Return true in case an alias was applied during the merge process.
#[instrument(level = "trace", skip_all)]
pub(crate) fn perform_fetch_step_merge<'a>(
    target_index: NodeIndex,
    source_index: NodeIndex,
    fetch_graph: &mut FetchGraph<'a, MultiTypeFetchStep>,
    force_merge_inputs: bool,
) -> Result<(), FetchGraphError> {
    let (target, source) = fetch_graph.get_pair_of_steps_mut(target_index, source_index)?;

    trace!(
        "merging fetch steps [{}] + [{}]",
        target_index.index(),
        source_index.index(),
    );

    let source_condition_merged = merge_source_condition_into_non_entity_target(target, source)?;
    if !source_condition_merged {
        target.scope_fetch_conditions_before_merge(source);
    }

    let scoped_aliases = target.output.safe_migrate_from_another(
        &source.output,
        &source.response_path.slice_from(target.response_path.len()),
        (
            target.flags.contains(FetchStepFlags::USED_FOR_REQUIRES),
            source.flags.contains(FetchStepFlags::USED_FOR_REQUIRES),
        ),
    )?;

    if !scoped_aliases.is_empty() {
        trace!(
            "Total of {} alises applied during safe merge of selections",
            scoped_aliases.len()
        );
        // In cases where merging a step resulted in internal aliasing, keep a record of the aliases.
        target.internal_aliases_locations.extend(scoped_aliases);
    }

    if let Some(input_rewrites) = source.input_rewrites.take() {
        if !input_rewrites.is_empty() {
            for input_rewrite in input_rewrites {
                target.add_input_rewrite(input_rewrite);
            }
        }
    }

    if force_merge_inputs {
        target
            .input
            .migrate_from_another(&source.input, &MergePath::default())?;
    } else if target.input.selecting_same_types(&source.input) {
        // It's safe to not check if a condition was turned into an inline fragment,
        // because if a condition is present and "me" is a non-entity fetch step,
        // then the type_name values of the inputs are different.
        if target.response_path != source.response_path {
            return Err(FetchGraphError::MismatchedResponsePath);
        }

        target
            .input
            .migrate_from_another(&source.input, &MergePath::default())?;
    }

    // Conditions may have been pushed down to keep the merge correct.
    // If the merged fetch is still guarded by one shared condition, lift it back to
    // step level.
    target.lift_shared_output_condition_to_fetch();

    let mut children_indexes: Vec<NodeIndex> = vec![];
    let mut parents_indexes: Vec<NodeIndex> = vec![];
    for edge_ref in fetch_graph.children_of(source_index) {
        children_indexes.push(edge_ref.target().id());
    }

    for edge_ref in fetch_graph.parents_of(source_index) {
        // We ignore self_index
        if edge_ref.source().id() != target_index {
            parents_indexes.push(edge_ref.source().id());
        }
    }

    // Replace parents:
    // 1. Add self -> child
    for child_index in children_indexes.iter() {
        fetch_graph.connect(target_index, *child_index);
    }
    // 2. Add parent -> self
    for parent_index in parents_indexes {
        fetch_graph.connect(parent_index, target_index);
    }
    // 3. Drop other -> child and parent -> other
    fetch_graph.remove_step(source_index);

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
pub fn is_reachable_via_alternative_upstream_path<'a>(
    graph: &FetchGraph<'a, MultiTypeFetchStep>,
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

impl FetchStepData<'_, MultiTypeFetchStep> {
    pub fn can_merge(
        &self,
        self_index: NodeIndex,
        other_index: NodeIndex,
        other: &Self,
        fetch_graph: &FetchGraph<'_, MultiTypeFetchStep>,
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
