use std::collections::{HashSet, VecDeque};

use petgraph::{graph::NodeIndex, visit::EdgeRef};
use tracing::{instrument, trace};

use crate::query_planner::{
    ast::{
        merge_path::MergePath,
        selection_set::{find_arguments_conflicts, find_selection_set_by_path},
    },
    planner::fetch::{
        error::FetchGraphError,
        fetch_graph::FetchGraph,
        fetch_step_data::{FetchStepData, FetchStepKind},
        selections::{FetchStepSelections, FetchStepSelectionsError},
    },
    state::supergraph_state::SupergraphState,
};

/// Handles the "target is non-entity, source has step-level condition" case.
/// When merging an entity fetch into a non-entity target, the condition must
/// stay attached to the source branch data, not to the whole target step.
fn merge_source_condition_into_non_entity_target(
    target: &FetchStepData,
    source: &mut FetchStepData,
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

    // The fields the source goes under may already have the condition, anywhere on the way
    // down from the target.
    let condition_redundant = source
        .response_path
        .strip_prefix(&target.response_path)
        .is_some_and(|path| path.has_condition(&condition));

    if !condition_redundant {
        source.output.wrap_with_condition(condition);
    }

    Ok(true)
}

/// Turns the result of a merge into a yes or no. Only a field conflict, or a type with
/// nowhere to go, means the steps can't be merged. Anything else is a bug, so it's passed up.
fn merge_succeeded(result: Result<(), FetchGraphError>) -> Result<bool, FetchGraphError> {
    match result {
        Ok(()) => Ok(true),
        Err(FetchGraphError::SelectionSetManipulationError(
            FetchStepSelectionsError::UnresolvableConflict(_)
            | FetchStepSelectionsError::NoSelectionForType(_),
        )) => Ok(false),
        Err(err) => Err(err),
    }
}

/// Moves everything `source` fetches into `target`. It only touches the two steps, not the
/// graph.
fn merge_step_data(
    target: &mut FetchStepData,
    source: &mut FetchStepData,
    force_merge_inputs: bool,
    supergraph: &SupergraphState,
) -> Result<(), FetchGraphError> {
    // Where the source's objects sit in the target. Batching puts steps together that sit at
    // the same place and differ only in type conditions, see `can_be_batched_with`.
    let source_fetch_path = if force_merge_inputs {
        MergePath::default()
    } else {
        source
            .response_path
            .strip_prefix(&target.response_path)
            .ok_or(FetchGraphError::MismatchedResponsePath)?
    };

    let source_condition_merged = merge_source_condition_into_non_entity_target(target, source)?;
    if !source_condition_merged {
        target.scope_fetch_conditions_before_merge(source);
    }
    target
        .output
        .safe_migrate_from_another(&source.output, &source_fetch_path, supergraph)?;

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
        if source_fetch_path.is_empty() {
            target
                .input
                .migrate_from_another(&source.input, &MergePath::default())?;
        }
        // Otherwise we pulled in a step from deeper in the response. Our own output
        // already has its input - that is why we could pull it in - so there is
        // nothing to copy over. See `can_absorb_nested_entity_call`.
    }

    // The merged step runs the mutations of both. Only neighbouring mutations are merged, so
    // remembering the last one keeps the one after it a neighbour.
    target.mutation_field_position = target
        .mutation_field_position
        .max(source.mutation_field_position);

    // Conditions may have been pushed down to keep the merge correct.
    // If the merged fetch is still guarded by one shared condition, lift it back to
    // step level.
    target.lift_shared_output_condition_to_fetch();

    Ok(())
}

/// Merges `source` into `target`, when their selections fit together, and returns whether
/// it did. They don't when a type of `source` has nowhere to go in `target`, which the types
/// alone tell. Or when two of their fields share a response key on the same objects, which
/// only happens when the client's fields do (`FetchGraph::client_keys_clash`). Only then is
/// the merge tried on copies first, the merge itself knows best where every field lands.
#[instrument(level = "trace", skip_all)]
pub(crate) fn try_merge_steps(
    target_index: NodeIndex,
    source_index: NodeIndex,
    fetch_graph: &mut FetchGraph,
    force_merge_inputs: bool,
    supergraph: &SupergraphState,
) -> Result<bool, FetchGraphError> {
    let may_clash = fetch_graph.client_keys_clash;
    let (target, source) = fetch_graph.get_pair_of_steps_mut(target_index, source_index)?;
    if !force_merge_inputs && !types_fit(target, source) {
        trace!(
            "fetch steps [{}] + [{}] have types that don't fit together",
            target_index.index(),
            source_index.index(),
        );
        return Ok(false);
    }

    if may_clash {
        let mut merged = target.clone();
        if force_merge_inputs {
            merged.declare_types_of(source);
        }
        if !merge_succeeded(merge_step_data(
            &mut merged,
            &mut source.clone(),
            force_merge_inputs,
            supergraph,
        ))? {
            trace!(
                "fetch steps [{}] + [{}] have fields that don't fit together",
                target_index.index(),
                source_index.index(),
            );
            return Ok(false);
        }
        *target = merged;
    } else {
        if force_merge_inputs {
            // Batching keeps every type's selections apart, so make room for the source's types.
            target.declare_types_of(source);
        }
        merge_step_data(target, source, force_merge_inputs, supergraph)?;
    }

    trace!(
        "merged fetch steps [{}] + [{}]",
        target_index.index(),
        source_index.index(),
    );
    fetch_graph.replace_step(source_index, target_index);

    Ok(true)
}

/// Does every type `source` fetches have a place in `target`? See `merge_step_data`, and
/// `FetchStepSelections::merge_target`.
fn types_fit(target: &FetchStepData, source: &FetchStepData) -> bool {
    let Some(fetch_path) = source.response_path.strip_prefix(&target.response_path) else {
        // Not a merge `can_merge` allows. `merge_step_data` says so.
        return true;
    };
    // A conditional entity call going into a query puts its input next to its output first.
    let input_goes_to_output = source.condition.is_some() && !target.is_entity_call();
    source.output.iter_selections().all(|(type_name, _)| {
        target
            .output
            .merge_target(type_name, &fetch_path, true)
            .is_ok()
    }) && (!input_goes_to_output
        || source.input.iter_selections().all(|(type_name, _)| {
            source
                .output
                .merge_target(type_name, &MergePath::default(), false)
                .is_ok()
        }))
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

impl FetchStepData {
    /// Could `other` go into this step, as far as the graph and the steps' kinds and places
    /// go? Whether their selections fit together is up to `try_merge_steps`.
    pub fn can_merge(
        &self,
        self_index: NodeIndex,
        other_index: NodeIndex,
        other: &Self,
        fetch_graph: &FetchGraph,
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
            if self.response_path != other.response_path
                && !(is_only_parent && self.can_absorb_nested_entity_call(other))
            {
                return false;
            }
        } else {
            // otherwise we can merge
            if !other.response_path.is_within(&self.response_path) {
                return false;
            }
        }

        if self.has_arguments_conflicts_with(other) {
            return false;
        }

        // if they do not share parents, they can't be merged
        if !is_only_parent
            && !fetch_graph.parents_of(self_index).all(|self_edge| {
                fetch_graph
                    .parents_of(other_index)
                    .any(|other_edge| other_edge.source() == self_edge.source())
            })
        {
            return false;
        }

        true
    }

    /// Makes room for every type of `other`, so its selections can sit next to ours
    /// instead of going into one of our types.
    fn declare_types_of(&mut self, other: &Self) {
        for (input_type_name, _) in other.input.iter_selections() {
            self.input.declare_known_type(input_type_name);
        }
        for (output_type_name, _) in other.output.iter_selections() {
            self.output.declare_known_type(output_type_name);
        }
    }

    /// Only call this when we are the only step `other` waits for.
    /// If something else feeds it, the keys it sends may come from there,
    /// and pointing its other parents at us could change the order of the graph,
    /// or add a cycle.
    fn can_absorb_nested_entity_call(&self, other: &Self) -> bool {
        let Some(path) = other.response_path.strip_prefix(&self.response_path) else {
            return false;
        };

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
        self.output
            .selections_for_definition(output_type)
            .and_then(|output| find_selection_set_by_path(output, &path))
            .is_some_and(|at_path| at_path.contains(input_selections))
    }

    pub fn has_arguments_conflicts_with(&self, other: &Self) -> bool {
        let input_conflicts = FetchStepSelections::iter_matching_types(
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
            fetch_step_data::{FetchStepData, FetchStepKind},
            selections::FetchStepSelections,
        },
        state::supergraph_state::{OperationKind, SubgraphName, SupergraphState},
        utils::parsing::{parse_operation, parse_schema},
    };

    use super::try_merge_steps;

    /// These tests don't depend on types, so the schema can be empty.
    fn supergraph() -> SupergraphState {
        SupergraphState::new(&parse_schema("type Query { a: Int }"))
    }

    /// Selections for one or more types, e.g. `&[("User", "{ id }"), ("Admin", "{ id }")]`.
    fn selections(types: &[(&str, &str)]) -> FetchStepSelections {
        let parse = |query: &str| -> SelectionSet {
            match parse_operation(query).definitions.first() {
                Some(Definition::Operation(OperationDefinition::SelectionSet(s))) => {
                    s.clone().into()
                }
                _ => panic!("expected a selection set"),
            }
        };

        let mut result = FetchStepSelections::new_empty();
        for (type_name, query) in types {
            let mut single = FetchStepSelections::new(type_name);
            single.add(&parse(query)).unwrap();
            result.declare_known_type(type_name);
            result
                .migrate_from_another(&single, &MergePath::default())
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

    fn add_entity_step(
        graph: &mut FetchGraph,
        service: &str,
        response_path: MergePath,
        input: FetchStepSelections,
        output: FetchStepSelections,
    ) -> petgraph::graph::NodeIndex {
        let response_path = graph.locations.get(&response_path);
        graph.add_step(FetchStepData {
            id: 0,
            service_name: SubgraphName(service.to_string()),
            response_path,
            input,
            output,
            kind: FetchStepKind::Entity,
            operation_kind: OperationKind::Query,
            condition: None,
            variable_usages: None,
            variable_definitions: None,
            mutation_field_position: None,
            input_rewrites: None,
            output_rewrites: None,
        })
    }

    /// A parent that fetches two types - what `batch_multi_type` builds - must not pull in a
    /// nested call. The merge only knows where to put the child's fields when the parent has a
    /// single type. Otherwise it looks for `Order` in the parent, and fails.
    #[test]
    fn multi_type_parent_does_not_absorb_nested_call() {
        let mut graph = FetchGraph::new(OperationKind::Query);

        let parent = add_entity_step(
            &mut graph,
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
        );
        let child = add_entity_step(
            &mut graph,
            "orders",
            path(&["accounts", "@", "orders", "@"]),
            selections(&[("Order", "{ __typename id }")]),
            selections(&[("Order", "{ sku }")]),
        );
        graph.connect(parent, child);

        let parent_data = graph.get_step_data(parent).unwrap();
        let child_data = graph.get_step_data(child).unwrap();
        assert!(!parent_data.can_merge(parent, child, child_data, &graph));
    }

    /// https://github.com/graphql-hive/router/issues/1308
    ///
    /// `Cat` and `Dog` calls batched into one step, next to an `Animal` call at the same path.
    /// The batched step has no `Animal` selections for the other one to go into. The other way
    /// around works, the `Animal` step takes them under `... on Cat` and `... on Dog`.
    #[test]
    fn multi_type_step_does_not_absorb_step_of_another_type() {
        let mut graph = FetchGraph::new(OperationKind::Query);

        let batched = add_entity_step(
            &mut graph,
            "catalog",
            path(&["listings", "@", "pet"]),
            selections(&[("Cat", "{ __typename id }"), ("Dog", "{ __typename id }")]),
            selections(&[("Cat", "{ whiskers }"), ("Dog", "{ tricks }")]),
        );
        let interface = add_entity_step(
            &mut graph,
            "catalog",
            path(&["listings", "@", "pet"]),
            selections(&[("Animal", "{ __typename id }")]),
            selections(&[("Animal", "{ __typename }")]),
        );

        // The merge doesn't work out, and leaves both steps as they were.
        assert!(!try_merge_steps(batched, interface, &mut graph, false, &supergraph()).unwrap());
        assert_eq!(graph.graph.node_count(), 2);
        assert!(graph
            .get_step_data(batched)
            .unwrap()
            .output
            .selections_for_definition("Animal")
            .is_none());

        assert!(try_merge_steps(interface, batched, &mut graph, false, &supergraph()).unwrap());
        assert_eq!(
            graph
                .get_step_data(interface)
                .unwrap()
                .output
                .selections_for_definition("Animal")
                .unwrap()
                .to_string(),
            "{__typename ...on Cat{whiskers} ...on Dog{tricks}}"
        );
    }

    /// Siblings `a` and `b` can't be merged, their inputs ask for `p` with different
    /// arguments. `c` fits with either. Once `a` takes `c` in, `b` has to be checked against
    /// `a` with `c`, not against `c` as it was. It used to be merged into `b` unchecked.
    #[test]
    fn sibling_merges_are_checked_against_merged_steps() {
        let mut graph = FetchGraph::new(OperationKind::Query);

        let root = add_entity_step(
            &mut graph,
            "products",
            path(&[]),
            selections(&[("Query", "{ __typename }")]),
            selections(&[("Query", "{ products { __typename id } }")]),
        );
        graph.root_index = Some(root);
        let sibling = |graph: &mut FetchGraph, input: &str, output: &str| {
            let step = add_entity_step(
                graph,
                "inventory",
                path(&["products", "@"]),
                selections(&[("Product", input)]),
                selections(&[("Product", output)]),
            );
            graph.connect(root, step);
            step
        };
        // The pass sees siblings in the reverse order they were added: `a`, `b`, `c`.
        sibling(&mut graph, "{ __typename id }", "{ c }");
        sibling(&mut graph, "{ __typename id p(x: 2) }", "{ b }");
        sibling(&mut graph, "{ __typename id p(x: 1) }", "{ a }");

        graph.merge_siblings(&supergraph()).unwrap();

        let mut inputs: Vec<String> = graph
            .step_indices()
            .filter(|index| *index != root)
            .map(|index| {
                graph
                    .get_step_data(index)
                    .unwrap()
                    .input
                    .selections_for_definition("Product")
                    .unwrap()
                    .to_string()
            })
            .collect();
        inputs.sort();
        assert_eq!(
            inputs,
            vec!["{__typename id p(x: 1)}", "{__typename id p(x: 2)}"]
        );
    }

    /// Two entity calls for the same type at unrelated paths can't be merged. The merged step
    /// would only fetch at `a.@`, and the entities at `b.@` would be lost.
    #[test]
    fn same_type_entity_calls_at_unrelated_paths_do_not_merge() {
        let mut graph = FetchGraph::new(OperationKind::Query);

        let a = add_entity_step(
            &mut graph,
            "catalog",
            path(&["a", "@"]),
            selections(&[("Order", "{ __typename id }")]),
            selections(&[("Order", "{ name }")]),
        );
        let b = add_entity_step(
            &mut graph,
            "catalog",
            path(&["b", "@"]),
            selections(&[("Order", "{ __typename id }")]),
            selections(&[("Order", "{ name }")]),
        );

        let result = try_merge_steps(a, b, &mut graph, false, &supergraph());
        assert!(
            matches!(result, Err(FetchGraphError::MismatchedResponsePath)),
            "{result:?}"
        );
    }
}
