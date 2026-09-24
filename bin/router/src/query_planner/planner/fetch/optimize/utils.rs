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
        fetch_step_data::{FetchStepData, FetchStepFlags, FetchStepKind, InternalAlias},
        selections::{FetchStepSelections, FetchStepSelectionsError},
        state::MultiTypeFetchStep,
    },
    state::supergraph_state::SupergraphState,
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

/// Turns the result of a trial merge into a yes or no. Only a field conflict, or a type with
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
/// graph, so it can run on copies too, to see if a merge works out.
fn merge_step_data(
    target: &mut FetchStepData<MultiTypeFetchStep>,
    source: &mut FetchStepData<MultiTypeFetchStep>,
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
    let scoped_aliases = target.output.safe_migrate_from_another(
        &source.output,
        &source_fetch_path,
        (
            target.flags.contains(FetchStepFlags::USED_FOR_REQUIRES),
            source.flags.contains(FetchStepFlags::USED_FOR_REQUIRES),
        ),
        supergraph,
    )?;

    trace!(
        "Total of {} alises applied during safe merge of selections",
        scoped_aliases.len()
    );
    target.internal_aliases.extend(
        scoped_aliases
            .into_iter()
            .map(|(path, alias)| InternalAlias {
                location: target.response_path.concat(&path),
                alias,
            }),
    );
    // Aliases the source made in earlier merges. Their locations are in the response,
    // so they stay the same.
    target.internal_aliases.append(&mut source.internal_aliases);

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

    // Conditions may have been pushed down to keep the merge correct.
    // If the merged fetch is still guarded by one shared condition, lift it back to
    // step level.
    target.lift_shared_output_condition_to_fetch();

    Ok(())
}

// Return true in case an alias was applied during the merge process.
#[instrument(level = "trace", skip_all)]
pub(crate) fn perform_fetch_step_merge(
    target_index: NodeIndex,
    source_index: NodeIndex,
    fetch_graph: &mut FetchGraph<MultiTypeFetchStep>,
    force_merge_inputs: bool,
    supergraph: &SupergraphState,
) -> Result<(), FetchGraphError> {
    trace!(
        "merging fetch steps [{}] + [{}]",
        target_index.index(),
        source_index.index(),
    );

    let (target, source) = fetch_graph.get_pair_of_steps_mut(target_index, source_index)?;
    merge_step_data(target, source, force_merge_inputs, supergraph)?;

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
        supergraph: &SupergraphState,
    ) -> Result<bool, FetchGraphError> {
        if self_index == other_index {
            return Ok(false);
        }

        if self.service_name != other.service_name {
            return Ok(false);
        }

        // We allow to merge root with entity calls by adding an inline fragment with the @include/@skip
        if self.is_entity_call() && other.is_entity_call() && self.condition != other.condition {
            return Ok(false);
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
                return Ok(false);
            }
        } else {
            // otherwise we can merge
            if !other.response_path.starts_with(&self.response_path) {
                return Ok(false);
            }
        }

        if self.has_arguments_conflicts_with(other) {
            return Ok(false);
        }

        // if they do not share parents, they can't be merged
        if !is_only_parent
            && !fetch_graph.parents_of(self_index).all(|self_edge| {
                fetch_graph
                    .parents_of(other_index)
                    .any(|other_edge| other_edge.source() == self_edge.source())
            })
        {
            return Ok(false);
        }

        self.merges_cleanly_with(other, supergraph)
    }

    /// Runs the merge on copies of both steps. Only the merge itself knows all the ways it
    /// can fail, like two plain fields it can't alias, or a type with nowhere to go.
    pub fn merges_cleanly_with(
        &self,
        other: &Self,
        supergraph: &SupergraphState,
    ) -> Result<bool, FetchGraphError> {
        merge_succeeded(merge_step_data(
            &mut self.clone(),
            &mut other.clone(),
            false,
            supergraph,
        ))
    }

    /// Makes room for every type of `other`, so its selections can sit next to ours
    /// instead of going into one of our types.
    pub fn declare_types_of(&mut self, other: &Self) {
        for (input_type_name, _) in other.input.iter_selections() {
            self.input.declare_known_type(input_type_name);
        }
        for (output_type_name, _) in other.output.iter_selections() {
            self.output.declare_known_type(output_type_name);
        }
    }

    /// Like `merges_cleanly_with`, for batching, where every type keeps its own selections.
    pub fn batches_cleanly_with(
        &self,
        other: &Self,
        supergraph: &SupergraphState,
    ) -> Result<bool, FetchGraphError> {
        let mut me = self.clone();
        me.declare_types_of(other);
        merge_succeeded(merge_step_data(
            &mut me,
            &mut other.clone(),
            true,
            supergraph,
        ))
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
            fetch_step_data::{FetchStepData, FetchStepFlags, FetchStepKind, InternalAlias},
            selections::FetchStepSelections,
            state::{MultiTypeFetchStep, SingleTypeFetchStep},
        },
        state::supergraph_state::{OperationKind, SubgraphName, SupergraphState},
        utils::parsing::{parse_operation, parse_schema},
    };

    use super::perform_fetch_step_merge;

    /// These tests don't depend on types, so the schema can be empty.
    fn supergraph() -> SupergraphState {
        SupergraphState::new(&parse_schema("type Query { a: Int }"))
    }

    /// For the alias tests, which need to know that `Cat` and `Dog` are `Node`s.
    fn pets_supergraph() -> SupergraphState {
        SupergraphState::new(&parse_schema(
            r#"
            directive @join__type(graph: join__Graph!, key: join__FieldSet) repeatable on OBJECT | INTERFACE
            directive @join__implements(graph: join__Graph!, interface: String!) repeatable on OBJECT | INTERFACE
            scalar join__FieldSet
            enum join__Graph { A @join__graph(name: "a", url: "") }
            type Query @join__type(graph: A) { things: [Node!]! }
            interface Node @join__type(graph: A) { id: ID! price: Int! }
            type Cat implements Node @join__type(graph: A) @join__implements(graph: A, interface: "Node") { id: ID! price: Int! }
            type Dog implements Node @join__type(graph: A) @join__implements(graph: A, interface: "Node") { id: ID! price: Int! }
            "#,
        ))
    }

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

    fn path_under_types(types: &[&str]) -> MergePath {
        types
            .iter()
            .fold(path(&["things", "@"]), |path, type_name| {
                path.push(Segment::TypeCondition(
                    std::collections::BTreeSet::from([(*type_name).to_string()]),
                    None,
                ))
            })
    }

    fn price_alias(types: &[&str]) -> InternalAlias {
        InternalAlias {
            location: path_under_types(types).push(Segment::Field(
                FieldPathSegment::named("price".to_string()),
                0,
                None,
            )),
            alias: "_internal_qp_alias_0".to_string(),
        }
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
            internal_aliases: Vec::new(),
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
        assert!(!parent_data
            .can_merge(parent, child, child_data, &graph, &supergraph())
            .unwrap());
    }

    /// https://github.com/graphql-hive/router/issues/1308
    ///
    /// `Cat` and `Dog` calls batched into one step, next to an `Animal` call at the same path.
    /// The batched step has no `Animal` selections for the other one to go into. The other way
    /// around works, the `Animal` step takes them under `... on Cat` and `... on Dog`.
    #[test]
    fn multi_type_step_does_not_absorb_step_of_another_type() {
        let mut graph =
            FetchGraph::<SingleTypeFetchStep>::new(OperationKind::Query).to_multi_type();

        let batched = graph.add_step(entity_step(
            "catalog",
            path(&["listings", "@", "pet"]),
            selections(&[("Cat", "{ __typename id }"), ("Dog", "{ __typename id }")]),
            selections(&[("Cat", "{ whiskers }"), ("Dog", "{ tricks }")]),
        ));
        let interface = graph.add_step(entity_step(
            "catalog",
            path(&["listings", "@", "pet"]),
            selections(&[("Animal", "{ __typename id }")]),
            selections(&[("Animal", "{ __typename }")]),
        ));

        let batched_data = graph.get_step_data(batched).unwrap();
        let interface_data = graph.get_step_data(interface).unwrap();
        assert!(!batched_data
            .can_merge(batched, interface, interface_data, &graph, &supergraph())
            .unwrap());
        assert!(interface_data
            .can_merge(interface, batched, batched_data, &graph, &supergraph())
            .unwrap());

        perform_fetch_step_merge(interface, batched, &mut graph, false, &supergraph()).unwrap();
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

        let result = perform_fetch_step_merge(a, b, &mut graph, false, &supergraph());
        assert!(
            matches!(result, Err(FetchGraphError::MismatchedResponsePath)),
            "{result:?}"
        );
    }

    /// Alias records hold response locations, so they come along unchanged when the aliased
    /// step is merged, and again when the result is merged into another step.
    #[test]
    fn alias_records_keep_their_location_across_merges() {
        let mut graph =
            FetchGraph::<SingleTypeFetchStep>::new(OperationKind::Query).to_multi_type();

        let users = graph.add_step(entity_step(
            "orders",
            path(&["user"]),
            selections(&[("User", "{ __typename id }")]),
            selections(&[("User", "{ orders { __typename id } }")]),
        ));
        let orders = graph.add_step(entity_step(
            "orders",
            path(&["user", "orders", "@"]),
            selections(&[("Order", "{ __typename id }")]),
            selections(&[("Order", "{ price _internal_qp_alias_0: price }")]),
        ));
        let other_users = graph.add_step(entity_step(
            "orders",
            path(&["user"]),
            selections(&[("User", "{ __typename id }")]),
            selections(&[("User", "{ name }")]),
        ));
        graph.connect(users, orders);
        let expected = vec![InternalAlias {
            location: path(&["user", "orders", "@", "price"]),
            alias: "_internal_qp_alias_0".to_string(),
        }];
        graph.get_step_data_mut(orders).unwrap().internal_aliases = expected.clone();

        perform_fetch_step_merge(users, orders, &mut graph, false, &supergraph()).unwrap();
        assert_eq!(
            graph.get_step_data(users).unwrap().internal_aliases,
            expected
        );

        perform_fetch_step_merge(other_users, users, &mut graph, false, &supergraph()).unwrap();
        assert_eq!(
            graph.get_step_data(other_users).unwrap().internal_aliases,
            expected
        );
    }

    /// `price` is aliased for `Cat`s only. A step reading `price` of `Dog`s at the same place
    /// reads other objects, so it keeps the plain name.
    #[test]
    fn alias_patching_keeps_to_its_type_branch() {
        let mut graph =
            FetchGraph::<SingleTypeFetchStep>::new(OperationKind::Query).to_multi_type();
        let under = |types: &str| {
            path(&["things", "@"]).push(Segment::TypeCondition(
                std::collections::BTreeSet::from([types.to_string()]),
                None,
            ))
        };

        let aliased = graph.add_step(entity_step(
            "shop",
            path(&["things", "@"]),
            selections(&[("Thing", "{ __typename id }")]),
            selections(&[("Thing", "{ ... on Cat { _internal_qp_alias_0: price } }")]),
        ));
        graph.get_step_data_mut(aliased).unwrap().internal_aliases = vec![InternalAlias {
            location: under("Cat").push(Segment::Field(
                FieldPathSegment::named("price".to_string()),
                0,
                None,
            )),
            alias: "_internal_qp_alias_0".to_string(),
        }];
        let cats = graph.add_step(entity_step(
            "pricing",
            under("Cat"),
            selections(&[("Cat", "{ __typename id price }")]),
            selections(&[("Cat", "{ tax }")]),
        ));
        let dogs = graph.add_step(entity_step(
            "pricing",
            under("Dog"),
            selections(&[("Dog", "{ __typename id price }")]),
            selections(&[("Dog", "{ tax }")]),
        ));
        graph.connect(aliased, cats);
        graph.connect(aliased, dogs);

        graph
            .apply_internal_aliases_patching(&pets_supergraph())
            .unwrap();

        let input = |step, type_name| {
            graph
                .get_step_data(step)
                .unwrap()
                .input
                .selections_for_definition(type_name)
                .unwrap()
                .to_string()
        };
        assert_eq!(
            input(cats, "Cat"),
            "{__typename id price: _internal_qp_alias_0}"
        );
        assert_eq!(input(dogs, "Dog"), "{__typename id price}");
    }

    /// Nested fragments narrow the runtime type cumulatively: Node + Cat is disjoint from
    /// Node + Dog, even though both paths mention Node.
    #[test]
    fn alias_patching_does_not_cross_nested_disjoint_type_conditions() {
        let mut graph =
            FetchGraph::<SingleTypeFetchStep>::new(OperationKind::Query).to_multi_type();

        let aliased = graph.add_step(entity_step(
            "shop",
            path(&["things", "@"]),
            selections(&[("Thing", "{ __typename id }")]),
            selections(&[(
                "Thing",
                "{ ... on Node { ... on Cat { _internal_qp_alias_0: price } } }",
            )]),
        ));
        graph.get_step_data_mut(aliased).unwrap().internal_aliases =
            vec![price_alias(&["Node", "Cat"])];

        let dogs = graph.add_step(entity_step(
            "pricing",
            path_under_types(&["Node", "Dog"]),
            selections(&[("Dog", "{ __typename id price }")]),
            selections(&[("Dog", "{ tax }")]),
        ));
        graph.connect(aliased, dogs);

        graph
            .apply_internal_aliases_patching(&pets_supergraph())
            .unwrap();

        assert_eq!(
            graph
                .get_step_data(dogs)
                .unwrap()
                .input
                .selections_for_definition("Dog")
                .unwrap()
                .to_string(),
            "{__typename id price}"
        );
    }

    /// Cat implements Node, so a reader under Cat can consume a field aliased under Node.
    #[test]
    fn alias_patching_matches_interface_and_implementing_object() {
        let mut graph =
            FetchGraph::<SingleTypeFetchStep>::new(OperationKind::Query).to_multi_type();

        let aliased = graph.add_step(entity_step(
            "shop",
            path(&["things", "@"]),
            selections(&[("Thing", "{ __typename id }")]),
            selections(&[("Thing", "{ ... on Node { _internal_qp_alias_0: price } }")]),
        ));
        graph.get_step_data_mut(aliased).unwrap().internal_aliases = vec![price_alias(&["Node"])];

        let cats = graph.add_step(entity_step(
            "pricing",
            path_under_types(&["Cat"]),
            selections(&[("Cat", "{ __typename id price }")]),
            selections(&[("Cat", "{ tax }")]),
        ));
        graph.connect(aliased, cats);

        graph
            .apply_internal_aliases_patching(&pets_supergraph())
            .unwrap();

        assert_eq!(
            graph
                .get_step_data(cats)
                .unwrap()
                .input
                .selections_for_definition("Cat")
                .unwrap()
                .to_string(),
            "{__typename id price: _internal_qp_alias_0}"
        );
    }
}
