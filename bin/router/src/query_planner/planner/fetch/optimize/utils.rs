use std::collections::{HashSet, VecDeque};

use petgraph::{
    graph::NodeIndex,
    visit::{EdgeRef, NodeRef},
};
use tracing::{instrument, trace};

use crate::query_planner::{
    ast::{
        merge_path::{MergePath, Segment},
        selection_set::{find_arguments_conflicts, find_selection_set_by_path},
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

    let source_condition_merged = merge_source_condition_into_non_entity_target(target, source)?;
    if !source_condition_merged {
        target.scope_fetch_conditions_before_merge(source);
    }

    let source_fetch_path = source.response_path.slice_from(target.response_path.len());
    let scoped_aliases = target.output.safe_migrate_from_another(
        &source.output,
        &source_fetch_path,
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

    // The source may have made aliases in earlier merges. Its fields now sit at
    // `source_fetch_path` in the target, so its records have to start there too.
    let target_type = target.output.try_as_single().map(|t| t.to_string());
    for (type_name, records) in std::mem::take(&mut source.internal_aliases_locations) {
        target.internal_aliases_locations.push((
            target_type.clone().unwrap_or(type_name),
            records
                .into_iter()
                .map(|(alias_path, alias)| (source_fetch_path.concat(&alias_path), alias))
                .collect(),
        ));
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
        if target.response_path == source.response_path {
            target
                .input
                .migrate_from_another(&source.input, &MergePath::default())?;
        } else if !source.response_path.starts_with(&target.response_path) {
            return Err(FetchGraphError::MismatchedResponsePath);
        }
        // Otherwise we pulled in a step from deeper in the response. Our own output
        // already has its input - that is why we could pull it in - so there is
        // nothing to copy over. See `can_absorb_nested_entity_call`.
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

        // Is `this` FetchStep the only one `other` waits for?
        let is_only_parent = fetch_graph.parents_of(other_index).count() == 1
            && fetch_graph
                .parents_of(other_index)
                .all(|edge| edge.source() == self_index);

        // If both are entities, their response_paths should match,
        // as we can't merge entity calls resolving different entities.
        // The one exception is a nested entity call that we feed ourselves,
        // see `can_absorb_nested_entity_call`.
        if matches!(self.kind, FetchStepKind::Entity) && self.kind == other.kind {
            if !self.response_path.eq(&other.response_path)
                && !(is_only_parent && self.can_absorb_nested_entity_call(other))
            {
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

        if is_only_parent {
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

    /// Only call this when we are the only step `other` waits for.
    /// If something else feeds it, the keys it sends may come from there,
    /// and pointing its other parents at us could change the order of the graph,
    /// or add a cycle.
    fn can_absorb_nested_entity_call(&self, other: &Self) -> bool {
        if !other.response_path.starts_with(&self.response_path) {
            return false;
        }

        let Some(input_type) = other.input.try_as_single() else {
            return false;
        };
        let Some(input_selections) = other.input.selections_for_definition(input_type) else {
            return false;
        };

        // The merge only knows where to put `other`'s fields when we fetch a single type.
        let Some(output_type) = self.output.try_as_single() else {
            return false;
        };

        // We must already have `other`'s input at that path.
        // If we do not, the fields we move here would have no object to sit in.
        let path = other.response_path.slice_from(self.response_path.len());
        self.output
            .selections_for_definition(output_type)
            .and_then(|output| find_selection_set_by_path(output, &path))
            .is_some_and(|at_path| at_path.contains(input_selections))
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

#[cfg(test)]
mod tests {
    use graphql_tools::parser::query::{Definition, OperationDefinition};

    use crate::query_planner::{
        ast::{
            merge_path::{FieldPathSegment, MergePath, Segment},
            selection_set::SelectionSet,
        },
        planner::fetch::{
            error::FetchGraphError,
            fetch_graph::FetchGraph,
            fetch_step_data::{FetchStepData, FetchStepFlags, FetchStepKind},
            selections::FetchStepSelections,
            state::{MultiTypeFetchStep, SingleTypeFetchStep},
        },
        state::supergraph_state::{OperationKind, SubgraphName},
        utils::parsing::parse_operation,
    };

    use super::perform_fetch_step_merge;

    /// Selections for one or more types, e.g. `&[("User", "{ id }"), ("Admin", "{ id }")]`.
    fn selections(types: &[(&str, &str)]) -> FetchStepSelections<MultiTypeFetchStep> {
        let parse = |query: &str| -> SelectionSet {
            match parse_operation(query).definitions.first() {
                Some(Definition::Operation(OperationDefinition::SelectionSet(s))) => {
                    s.clone().into()
                }
                _ => panic!("expected a selection set"),
            }
        };

        let mut result = FetchStepSelections::<SingleTypeFetchStep>::new_empty().into_multi_type();
        for (type_name, query) in types {
            let mut single = FetchStepSelections::<SingleTypeFetchStep>::new(type_name);
            single.add(&parse(query)).unwrap();
            result.declare_known_type(type_name);
            result
                .migrate_from_another(&single.into_multi_type(), &MergePath::default())
                .unwrap();
        }
        result
    }

    fn path(fields: &[&str]) -> MergePath {
        MergePath::new(
            fields
                .iter()
                .map(|name| match *name {
                    "@" => Segment::List,
                    name => Segment::Field(FieldPathSegment::named(name.to_string()), 0, None),
                })
                .collect(),
        )
    }

    fn entity_step(
        service: &str,
        response_path: MergePath,
        input: FetchStepSelections<MultiTypeFetchStep>,
        output: FetchStepSelections<MultiTypeFetchStep>,
    ) -> FetchStepData<MultiTypeFetchStep> {
        FetchStepData {
            id: 0,
            service_name: SubgraphName(service.to_string()),
            response_path,
            input,
            output,
            kind: FetchStepKind::Entity,
            operation_kind: OperationKind::Query,
            flags: FetchStepFlags::empty(),
            condition: None,
            variable_usages: None,
            variable_definitions: None,
            mutation_field_position: None,
            input_rewrites: None,
            output_rewrites: None,
            internal_aliases_locations: Vec::new(),
        }
    }

    /// A parent that fetches two types - what `batch_multi_type` builds - must not pull in a
    /// nested call. The merge only knows where to put the child's fields when the parent has a
    /// single type. Otherwise it looks for `Order` in the parent, and fails.
    #[test]
    fn multi_type_parent_does_not_absorb_nested_call() {
        let mut graph =
            FetchGraph::<SingleTypeFetchStep>::new(OperationKind::Query).to_multi_type();

        let parent = graph.add_step(entity_step(
            "orders",
            path(&["accounts", "@"]),
            selections(&[
                ("User", "{ __typename id }"),
                ("Admin", "{ __typename id }"),
            ]),
            selections(&[
                ("User", "{ orders { __typename id } }"),
                ("Admin", "{ orders { __typename id } }"),
            ]),
        ));
        let child = graph.add_step(entity_step(
            "orders",
            path(&["accounts", "@", "orders", "@"]),
            selections(&[("Order", "{ __typename id }")]),
            selections(&[("Order", "{ sku }")]),
        ));
        graph.connect(parent, child);

        let parent_data = graph.get_step_data(parent).unwrap();
        let child_data = graph.get_step_data(child).unwrap();
        assert!(!parent_data.can_merge(parent, child, child_data, &graph));
    }

    /// Two entity calls for the same type at unrelated paths can't be merged. The merged step
    /// would only fetch at `a.@`, and the entities at `b.@` would be lost.
    #[test]
    fn same_type_entity_calls_at_unrelated_paths_do_not_merge() {
        let mut graph =
            FetchGraph::<SingleTypeFetchStep>::new(OperationKind::Query).to_multi_type();

        let a = graph.add_step(entity_step(
            "catalog",
            path(&["a", "@"]),
            selections(&[("Order", "{ __typename id }")]),
            selections(&[("Order", "{ name }")]),
        ));
        let b = graph.add_step(entity_step(
            "catalog",
            path(&["b", "@"]),
            selections(&[("Order", "{ __typename id }")]),
            selections(&[("Order", "{ name }")]),
        ));

        let result = perform_fetch_step_merge(a, b, &mut graph, false);
        assert!(
            matches!(result, Err(FetchGraphError::MismatchedResponsePath)),
            "{result:?}"
        );
    }

    /// A step that already made aliases in an earlier merge gets merged again. Its fields now sit
    /// under `orders.@` in the target, so its alias records have to move there too - otherwise
    /// the steps reading those fields never learn about the alias.
    #[test]
    fn merge_keeps_alias_records_of_the_merged_step() {
        let mut graph =
            FetchGraph::<SingleTypeFetchStep>::new(OperationKind::Query).to_multi_type();

        let target = graph.add_step(entity_step(
            "orders",
            path(&["user"]),
            selections(&[("User", "{ __typename id }")]),
            selections(&[("User", "{ orders { __typename id } }")]),
        ));
        let source = graph.add_step(entity_step(
            "orders",
            path(&["user", "orders", "@"]),
            selections(&[("Order", "{ __typename id }")]),
            selections(&[("Order", "{ price _internal_qp_alias_0: price }")]),
        ));
        graph.connect(target, source);
        graph
            .get_step_data_mut(source)
            .unwrap()
            .internal_aliases_locations
            .push((
                "Order".to_string(),
                vec![(path(&["price"]), "_internal_qp_alias_0".to_string())],
            ));

        perform_fetch_step_merge(target, source, &mut graph, false).unwrap();

        assert_eq!(
            graph
                .get_step_data(target)
                .unwrap()
                .internal_aliases_locations,
            vec![(
                "User".to_string(),
                vec![(
                    path(&["orders", "@", "price"]),
                    "_internal_qp_alias_0".to_string()
                )]
            )]
        );
    }
}
