use std::collections::{BTreeSet, HashSet, VecDeque};

use petgraph::{
    graph::NodeIndex,
    visit::{EdgeRef, NodeRef},
};
use tracing::{instrument, trace};

use crate::{
    ast::merge_path::Segment,
    ast::{merge_path::MergePath, selection_set::find_arguments_conflicts},
    planner::fetch::{
        error::FetchGraphError,
        fetch_graph::FetchGraph,
        fetch_step_data::{FetchStepData, FetchStepFlags, FetchStepKind},
        selections::FetchStepSelections,
        state::MultiTypeFetchStep,
    },
};

pub fn type_condition_types_from_response_path<'a>(
    response_path: &'a MergePath,
) -> Option<BTreeSet<&'a str>> {
    let conditioned_types = response_path
        .inner
        .iter()
        .filter_map(|segment| match segment {
            Segment::TypeCondition(type_names, _) => Some(type_names),
            _ => None,
        })
        .flat_map(|type_names| type_names.iter().map(|s| s.as_str()).clone())
        .collect::<BTreeSet<_>>();

    if conditioned_types.is_empty() {
        None
    } else {
        Some(conditioned_types)
    }
}

/// Moves a step-level condition into type-specific output branches when safe.
/// Prevents accidentally applying the condition to sibling types.
///
/// Example:
/// If a merged fetch has Book + Magazine output and a condition from a Book path,
/// this applies the condition to Book selections only.
fn scope_target_condition(target: &mut FetchStepData<MultiTypeFetchStep>) {
    if !target.is_entity_call() || !target.is_fetching_multiple_types() {
        return;
    }

    let Some(condition) = target.condition.clone() else {
        return;
    };

    let Some(conditioned_types) = type_condition_types_from_response_path(&target.response_path)
    else {
        return;
    };

    target
        .output
        .wrap_with_condition_for_types(condition, &conditioned_types);
    target.condition = None;
}

/// Handles the "target is non-entity, source has step-level condition" case.
/// When merging an entity fetch into a non-entity target, the condition must
/// stay attached to the source branch data, not to the whole target step.
fn merge_source_condition_into_non_entity_target(
    target: &FetchStepData<MultiTypeFetchStep>,
    source: &mut FetchStepData<MultiTypeFetchStep>,
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

/// Tries to scope `source.condition` to source type branches.
/// If type scope is unclear, keeps the condition on `target` as a safe fallback.
fn preserve_or_scope_source_condition_for_entity_target(
    target: &mut FetchStepData<MultiTypeFetchStep>,
    source: &mut FetchStepData<MultiTypeFetchStep>,
) {
    // This helper only applies when the merge target is an entity fetch
    if !target.is_entity_call() {
        return;
    }

    // If source has no step-level condition, there is nothing to scope or preserve
    let Some(condition) = source.condition.take() else {
        return;
    };

    // We can scope condition to concrete type branches only in multi-type step.
    // The response path's type-condition segments tell us which concrete types are affected.
    let conditioned_types =
        if target.is_fetching_multiple_types() || source.is_fetching_multiple_types() {
            type_condition_types_from_response_path(&source.response_path)
        } else {
            None
        };

    // We know the concrete types, so apply condition only to those
    // source output branches (instead of gating whole step).
    if let Some(types) = conditioned_types {
        source
            .output
            .wrap_with_condition_for_types(condition, &types);
        return;
    }

    // If type scope is unclear, keep condition at target step level.
    target.condition = Some(condition);
}

// Return true in case an alias was applied during the merge process.
#[instrument(level = "trace", skip_all)]
pub(crate) fn perform_fetch_step_merge(
    target_index: NodeIndex,
    source_index: NodeIndex,
    fetch_graph: &mut FetchGraph<MultiTypeFetchStep>,
    force_merge_inputs: bool,
) -> Result<(), FetchGraphError> {
    let (target, source) = fetch_graph.get_pair_of_steps_mut(target_index, source_index)?;

    trace!(
        "merging fetch steps [{}] + [{}]",
        target_index.index(),
        source_index.index(),
    );

    scope_target_condition(target);

    let source_condition_merged = merge_source_condition_into_non_entity_target(target, source)?;
    if !source_condition_merged {
        preserve_or_scope_source_condition_for_entity_target(target, source);
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
