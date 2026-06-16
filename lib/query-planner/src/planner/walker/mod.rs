mod best_path;
pub(crate) mod error;
mod excluded;
pub(crate) mod path;
pub(crate) mod pathfinder;
mod utils;

use std::collections::HashSet;
use std::collections::VecDeque;
use std::rc::Rc;

use crate::{
    ast::{
        merge_path::Condition,
        operation::OperationDefinition,
        selection_item::SelectionItem,
        selection_set::{FieldSelection, InlineFragmentSelection, SelectionSet},
    },
    graph::{
        edge::{Edge, PlannerOverrideContext},
        node::Node,
        Graph,
    },
    planner::walker::pathfinder::{find_self_referencing_direct_path, NavigationTarget},
    state::supergraph_state::{OperationKind, SupergraphState},
    utils::cancellation::CancellationToken,
};
use best_path::{find_best_paths, BestPathTracker};
use error::WalkOperationError;
use excluded::ExcludedFromLookup;
use path::OperationPath;
use pathfinder::{find_direct_path, find_direct_paths, find_indirect_paths};
use tracing::{instrument, span, trace, Level};
use utils::get_entrypoints;

pub struct ResolvedOperation<'graph> {
    pub operation_kind: OperationKind,
    pub root_field_groups: Vec<BestPathsPerLeaf<'graph>>,
}

// TODO: Make a better struct
pub type BestPathsPerLeaf<'graph> = Vec<Vec<OperationPath<'graph>>>;

// TODO: Consider to use VecDeque(fixed_size) if we can predict it?
// TODO: Consider to drop this IR layer and just go with QTP directly.

type WorkItem<'graph, 'op> = (&'op SelectionItem, Rc<Vec<OperationPath<'graph>>>);
type ResolutionStack<'graph, 'op> = Vec<WorkItem<'graph, 'op>>;

#[instrument(level = "trace", skip_all)]
pub fn walk_operation<'graph, 'op: 'graph>(
    graph: &'graph Graph,
    supergraph: &'graph SupergraphState,
    override_context: &'graph PlannerOverrideContext,
    operation: &'op OperationDefinition,
    cancellation_token: &'graph CancellationToken,
) -> Result<ResolvedOperation<'graph>, WalkOperationError> {
    let operation_kind = operation
        .operation_kind
        .clone()
        .unwrap_or(OperationKind::Query);
    let (op_type, selection_set) = operation.parts();
    trace!("operation is of type {:?}", op_type);

    let root_entrypoints = get_entrypoints(graph, op_type)?;
    let initial_paths: Rc<Vec<OperationPath<'graph>>> = Rc::new(
        root_entrypoints
            .iter()
            .map(|edge| OperationPath::new_entrypoint(edge))
            .collect(),
    );

    let mut paths_grouped_by_root_field: Vec<BestPathsPerLeaf> =
        Vec::with_capacity(operation.selection_set.items.len());

    // It's critical to iterate over root fiels and preserve their original order
    for selection_item in selection_set.items.iter() {
        let mut stack_to_resolve: VecDeque<WorkItem> = VecDeque::new();

        stack_to_resolve.push_back((selection_item, Rc::clone(&initial_paths)));

        let mut paths_per_leaf: Vec<Vec<OperationPath<'graph>>> = vec![];

        while let Some((selection_item, paths)) = stack_to_resolve.pop_front() {
            cancellation_token.bail_if_cancelled()?;
            let (next_stack_to_resolve, new_paths_per_leaf) = process_selection(
                graph,
                supergraph,
                override_context,
                selection_item,
                &paths,
                &[],
                cancellation_token,
            )?;

            paths_per_leaf.extend(new_paths_per_leaf);
            for item in next_stack_to_resolve.into_iter().rev() {
                stack_to_resolve.push_front(item);
            }
        }

        paths_grouped_by_root_field.push(paths_per_leaf);
    }

    Ok(ResolvedOperation {
        operation_kind,
        root_field_groups: paths_grouped_by_root_field,
    })
}

fn process_selection<'graph, 'op: 'graph>(
    graph: &'graph Graph,
    supergraph: &'graph SupergraphState,
    override_context: &'graph PlannerOverrideContext,
    selection_item: &'op SelectionItem,
    paths: &[OperationPath<'graph>],
    fields_to_resolve_locally: &[String],
    cancellation_token: &'graph CancellationToken,
) -> Result<
    (
        ResolutionStack<'graph, 'op>,
        Vec<Vec<OperationPath<'graph>>>,
    ),
    WalkOperationError,
> {
    let mut stack_to_resolve: ResolutionStack = vec![];
    let mut paths_per_leaf: Vec<Vec<OperationPath<'graph>>> = vec![];

    match selection_item {
        SelectionItem::InlineFragment(fragment) => {
            let (next_selection_items, new_paths_per_leaf) = process_inline_fragment(
                graph,
                supergraph,
                override_context,
                fragment,
                paths,
                fields_to_resolve_locally,
                cancellation_token,
            )?;
            paths_per_leaf.extend(new_paths_per_leaf);
            stack_to_resolve.extend(next_selection_items);
        }
        SelectionItem::Field(field) => {
            let (next_selection_items, new_paths_per_leaf) = process_field(
                graph,
                supergraph,
                override_context,
                field,
                paths,
                fields_to_resolve_locally,
                cancellation_token,
            )?;
            paths_per_leaf.extend(new_paths_per_leaf);
            stack_to_resolve.extend(next_selection_items);
        }
        SelectionItem::FragmentSpread(_) => {
            // No processing needed for FragmentSpread
        }
    }

    Ok((stack_to_resolve, paths_per_leaf))
}

#[instrument(level = "trace", skip_all)]
fn process_selection_set<'graph, 'op: 'graph>(
    graph: &'graph Graph,
    supergraph: &'graph SupergraphState,
    override_context: &'graph PlannerOverrideContext,
    selection_set: &'op SelectionSet,
    paths: &[OperationPath<'graph>],
    fields_to_resolve_locally: &[String],
    cancellation_token: &'graph CancellationToken,
) -> Result<
    (
        ResolutionStack<'graph, 'op>,
        Vec<Vec<OperationPath<'graph>>>,
    ),
    WalkOperationError,
> {
    let mut stack_to_resolve: ResolutionStack = vec![];
    let mut paths_per_leaf: Vec<Vec<OperationPath<'graph>>> = vec![];

    for item in selection_set.items.iter() {
        let (next_stack_to_resolve, new_paths_per_leaf) = process_selection(
            graph,
            supergraph,
            override_context,
            item,
            paths,
            fields_to_resolve_locally,
            cancellation_token,
        )?;
        paths_per_leaf.extend(new_paths_per_leaf);
        stack_to_resolve.extend(next_stack_to_resolve);
    }

    Ok((stack_to_resolve, paths_per_leaf))
}

#[instrument(level = "trace", skip_all, fields(
  type_condition = fragment.type_condition,
))]
fn process_inline_fragment<'graph, 'op: 'graph>(
    graph: &'graph Graph,
    supergraph: &'graph SupergraphState,
    override_context: &'graph PlannerOverrideContext,
    fragment: &'op InlineFragmentSelection,
    paths: &[OperationPath<'graph>],
    fields_to_resolve_locally: &[String],
    cancellation_token: &'graph CancellationToken,
) -> Result<
    (
        ResolutionStack<'graph, 'op>,
        Vec<Vec<OperationPath<'graph>>>,
    ),
    WalkOperationError,
> {
    trace!(
        "Processing inline fragment '{}' on type '{}' (skip: {:?}, include: {:?}) through {} possible paths",
        fragment.selections,
        fragment.type_condition,
        fragment.include_if,
        fragment.skip_if,
        paths.len()
    );

    cancellation_token.bail_if_cancelled()?;

    // if the current type is an object type we ignore an abstract move
    // but if it's a union, we need to find an abstract move to the target type
    let tail_index = graph.get_edge_tail(
        &paths
            .first()
            .unwrap()
            .last_segment
            .as_ref()
            .unwrap()
            .edge_index,
    )?;

    // if type condition if matching the tail's type name,
    // ignore the fragment and do not check if `... on X` is possible.
    // In case of
    //  - interfaces - `... on Interface` - we look for interface's fields.
    //  - object types - `... on Object`-  we look for object's fields.
    //  - union types - `... on Union` will cause a graphql validation error.
    // We don't need to worry about correctness here as it's handled by graphql validations.
    let tail = graph.node(tail_index)?;
    let tail_type_name = match tail {
        Node::SubgraphType(t) => &t.name,
        _ => panic!("Expected a subgraph type when resolving fragments"),
    };

    if tail_type_name == &fragment.type_condition {
        // It's the same type and no conditions are applied, we can skip the fragment processing
        // and go directly to its selections.
        if fragment.include_if.is_none() && fragment.skip_if.is_none() {
            return process_selection_set(
                graph,
                supergraph,
                override_context,
                &fragment.selections,
                paths,
                fields_to_resolve_locally,
                cancellation_token,
            );
        }

        // Looks like the fragment has conditions, we need to process them differently.
        // We aim to preserve the inline fragment due to conditions, instead of eliminating it,
        // and jumping straight to its selections.
        let condition: Option<Condition> = fragment.into();

        let mut next_paths: Vec<OperationPath<'graph>> = Vec::with_capacity(paths.len());
        for path in paths {
            let path_span = span!(
                Level::TRACE,
                "explore_path",
                path = path.pretty_print(graph)
            );
            let _enter = path_span.enter();

            // Find a direct path that references the same type as the current tail,
            let direct_path = find_self_referencing_direct_path(
                graph,
                override_context,
                path,
                &fragment.type_condition,
                condition.as_ref().expect("Condition should be present"),
                cancellation_token,
            )?;

            trace!("advanced: {}", path.pretty_print(graph));

            next_paths.push(direct_path);
        }

        // Now process the selections under the fragment using the advanced paths
        return process_selection_set(
            graph,
            supergraph,
            override_context,
            &fragment.selections,
            &next_paths,
            fields_to_resolve_locally,
            cancellation_token,
        );
    }

    trace!(
        "Trying to advance to: ... on {}, through {} possible paths",
        fragment.type_condition,
        paths.len()
    );

    let mut next_paths: Vec<OperationPath<'graph>> = Vec::with_capacity(paths.len());
    for path in paths {
        let path_span = span!(
            Level::TRACE,
            "explore_path",
            path = path.pretty_print(graph)
        );
        let _enter = path_span.enter();
        let direct_path = find_direct_path(
            graph,
            override_context,
            path,
            &NavigationTarget::ConcreteType(&fragment.type_condition, fragment.into()),
            cancellation_token,
        )?;

        let found_direct_path = direct_path.is_some();
        trace!("Direct path found: {}", found_direct_path);
        if let Some(direct_path) = direct_path {
            trace!("advanced: {}", path.pretty_print(graph));
            next_paths.push(direct_path);
        }

        if fields_to_resolve_locally.is_empty() {
            let mut indirect_paths = find_indirect_paths(
                graph,
                override_context,
                path,
                &NavigationTarget::ConcreteType(&fragment.type_condition, fragment.into()),
                &ExcludedFromLookup::new(),
                cancellation_token,
            )?;

            if !indirect_paths.is_empty() {
                trace!("advanced: {}", path.pretty_print(graph));
                next_paths.push(indirect_paths.remove(0));
            }

            if indirect_paths.is_empty() && !found_direct_path {
                // Looks like a union member or an interface implementation is not resolvable.
                // The fact the fragment for that object type passed GraphQL validations,
                // means that it's a child of the abstract type,
                // and it was probably eliminated from the Graph because of intersection.
                trace!(
                    "Object type '{}' is not resolvable by '{}', resolve only the __typename",
                    fragment.type_condition,
                    tail_type_name
                );
            }
        }
    }

    if next_paths.is_empty() {
        let mut tracker = BestPathTracker::new(graph);

        for path in paths {
            let path_span = span!(
                Level::TRACE,
                "explore_path",
                path = path.pretty_print(graph)
            );
            let _enter = path_span.enter();
            let direct_paths = find_direct_paths(
                graph,
                override_context,
                path,
                &NavigationTarget::Field {
                    field: &FieldSelection::new_typename(),
                    target_subgraph_ids: None,
                },
                cancellation_token,
            )?;

            trace!("Direct paths found: {}", direct_paths.len());

            if !direct_paths.is_empty() {
                for p in direct_paths {
                    tracker.add(&p)?;
                }
            } else {
                return Err(WalkOperationError::NoPathsFound("__typename".to_string()));
            }
        }

        let next_paths = tracker.get_best_paths();

        if next_paths.is_empty() {
            return Err(WalkOperationError::NoPathsFound("__typename".to_string()));
        }

        return Ok((vec![], vec![find_best_paths(next_paths)]));
    }

    process_selection_set(
        graph,
        supergraph,
        override_context,
        &fragment.selections,
        &next_paths,
        fields_to_resolve_locally,
        cancellation_token,
    )
}

/// The same union may have different members in different subgraphs.
/// For example:
///
/// - graph A: Action = Common | OnlyA
/// - graph B: Action = Common | OnlyB
///
/// If the planner still has valid candidate paths through both A and B,
/// the response shape must not depend on which path wins later.
/// In that case we keep only members that are available in every candidate graph.
///
/// In the example above, only `Common` is safe to keep.
fn narrow_partial_union_paths<'graph>(paths: &mut Vec<OperationPath<'graph>>) {
    if paths.len() <= 1 {
        return;
    }

    let Some(union_context) = paths.first().and_then(|path| path.union_context.as_ref()) else {
        return;
    };

    let all_same_field = paths.iter().all(|path| {
        path.union_context
            .as_ref()
            .is_some_and(|ctx| ctx.eq_field(union_context))
    });

    if !all_same_field {
        return;
    }

    // Collect the union members that each subgraph can return.
    //
    // All paths through the same graph share the same `possible_members` because they originate
    // from the same `UnionMembersData` (one per graph). We only need to record each graph's member
    // set once — no accumulation is required.
    type GraphId<'graph> = &'graph str;
    type MembersPerGraph<'graph> = Vec<(GraphId<'graph>, Vec<&'graph str>)>;
    let mut members_per_graph: MembersPerGraph<'graph> = Vec::new();

    for path in paths.iter() {
        let context = path
            .union_context
            .as_ref()
            // It's safe to unwrap as `all_same_field` checked that all paths have the same context
            .expect("union member context should exist at this point");

        if !members_per_graph
            .iter()
            .any(|(graph_id, _)| graph_id == &context.graph_id)
        {
            members_per_graph.push((context.graph_id, context.possible_members.clone()));
        }
    }

    if members_per_graph.len() <= 1 {
        return;
    }

    let (_, least_members_set) = members_per_graph
        .iter()
        .min_by_key(|(_, members)| members.len())
        // It's safe as we checked that `members_per_graph` has at least two entries above
        .expect("members_per_graph has at least two entries");

    // Start from the smallest member list.
    // Then shrink it with each graph until only shared members remain.
    let mut shared_members: Vec<&'graph str> = least_members_set.clone();

    for (_, members) in &members_per_graph {
        shared_members.retain(|member| members.contains(member));

        if shared_members.is_empty() {
            // No union member exists in every candidate subgraph.
            // None of these paths is safe to keep.
            paths.clear();
            return;
        }
    }

    paths.retain_mut(|path| {
        // It's safe to unwrap as we checked that `union_context` exists already
        let context = path
            .union_context
            .as_mut()
            .expect("union context should exist at this point");

        // Keep only paths whose current member is shared
        if !shared_members.contains(&context.member_name) {
            return false;
        }

        // Update the possible members to the shared members set.
        context.possible_members = shared_members.clone();
        true
    });
}

#[instrument(level = "trace", skip_all, fields(
  field_name = &field.name,
  leaf = field.is_leaf()
))]
fn process_field<'graph, 'op: 'graph>(
    graph: &'graph Graph,
    supergraph: &'graph SupergraphState,
    override_context: &'graph PlannerOverrideContext,
    field: &'op FieldSelection,
    paths: &[OperationPath<'graph>],
    _fields_to_resolve_locally: &[String],
    cancellation_token: &'graph CancellationToken,
) -> Result<
    (
        ResolutionStack<'graph, 'op>,
        Vec<Vec<OperationPath<'graph>>>,
    ),
    WalkOperationError,
> {
    let mut next_stack_to_resolve: ResolutionStack = vec![];
    let mut paths_per_leaf: Vec<Vec<OperationPath<'graph>>> = vec![];
    let mut tracker = BestPathTracker::new(graph);

    trace!(
        "Trying to advance to: {} through {} possible paths",
        field,
        paths.len()
    );

    cancellation_token.bail_if_cancelled()?;

    let mut target_subgraph_ids_cache = None;

    for path in paths {
        let path_span = span!(
            Level::TRACE,
            "explore_path",
            path = path.pretty_print(graph)
        );
        let _enter = path_span.enter();

        let mut advanced = false;

        let excluded = ExcludedFromLookup::new();
        let direct_paths = find_direct_paths(
            graph,
            override_context,
            path,
            &NavigationTarget::Field {
                field,
                target_subgraph_ids: None,
            },
            cancellation_token,
        )?;
        trace!("Direct paths found: {}", direct_paths.len());
        let found_direct_paths_to_leaf = !direct_paths.is_empty() && field.is_leaf();

        if !direct_paths.is_empty() {
            advanced = true;
            for direct_path in &direct_paths {
                tracker.add(direct_path)?;
            }
        }

        if !found_direct_paths_to_leaf {
            if target_subgraph_ids_cache.is_none() {
                target_subgraph_ids_cache =
                    Some(field_target_subgraph_ids(supergraph, field, paths, graph)?);
            }
            let target_subgraph_ids = target_subgraph_ids_cache
                .as_ref()
                .and_then(|ids| ids.as_ref());

            let indirect_paths = find_indirect_paths(
                graph,
                override_context,
                path,
                &NavigationTarget::Field {
                    field,
                    target_subgraph_ids,
                },
                &excluded,
                cancellation_token,
            )?;
            trace!("Indirect paths found: {}", indirect_paths.len());

            if !indirect_paths.is_empty() {
                advanced = true;
                for indirect_path in indirect_paths {
                    tracker.add(&indirect_path)?;
                }
            }
        }
        if found_direct_paths_to_leaf {
            trace!("Skipping indirect paths for leaf direct hit");
        }

        trace!(
            "{}: {}",
            if advanced {
                "advanced"
            } else {
                "failed to advance"
            },
            path.pretty_print(graph)
        );
    }

    let mut next_paths = tracker.get_best_paths();
    narrow_partial_union_paths(&mut next_paths);
    if next_paths.is_empty() {
        return Err(WalkOperationError::NoPathsFound(field.name.to_string()));
    }

    let mut fields_to_resolve_locally: Vec<String> = Vec::new();
    if !field.is_leaf() {
        let field_move_paths: Vec<_> = next_paths
            .iter()
            .filter(|path| {
                path.last_segment.as_ref().is_some_and(|seg| {
                    matches!(graph.edge(seg.edge_index).unwrap(), Edge::FieldMove(_))
                })
            })
            .collect();

        if !field_move_paths.is_empty() {
            let edge_index = field_move_paths[0]
                .last_segment
                .as_ref()
                .unwrap()
                .edge_index;

            let head_index = graph.get_edge_head(&edge_index)?;
            let tail_index = graph.get_edge_tail(&edge_index)?;

            let head = graph.node(head_index)?;
            let tail = graph.node(tail_index)?;

            let parent_type_name = head.name_str();
            let parent_def = supergraph
                .definitions
                .get(parent_type_name)
                .ok_or_else(|| WalkOperationError::TypeNotFound(parent_type_name.to_string()))?;

            let field_def = parent_def.fields().get(&field.name).ok_or_else(|| {
                WalkOperationError::FieldNotFound(
                    field.name.to_string(),
                    parent_type_name.to_string(),
                )
            })?;

            let output_type = supergraph
                .definitions
                .get(tail.name_str())
                .ok_or_else(|| WalkOperationError::TypeNotFound(tail.name_str().to_string()))?;

            if output_type.is_interface_type()
                && field_def.resolvable_in_graphs(parent_def).len() > 1
                // if there's one fragment, the query planner can decide which subgraph to use, based on the fragment's type condition
                && field
                    .selections
                    .items
                    .iter()
                    .filter(|item| matches!(item, SelectionItem::InlineFragment(_)))
                    .count()
                    > 1
            {
                fields_to_resolve_locally = output_type
                    .fields()
                    .keys()
                    .map(|name| name.to_string())
                    .collect();
            }
        }
    }

    if !fields_to_resolve_locally.is_empty() {
        let path_span = span!(
            Level::TRACE,
            "Shareable interface detected. Validating that sub-selections can be resolved from a single path."
        );
        let _enter = path_span.enter();
        let mut valid_paths_for_children: Vec<OperationPath<'graph>> =
            Vec::with_capacity(next_paths.len());
        for candidate_path in &next_paths {
            let mut all_children_resolvable = true;
            // We don't need the results of the sub-walk here, only whether it was successful
            for child_selection in &field.selections.items {
                let finding = process_selection(
                    graph,
                    supergraph,
                    override_context,
                    child_selection,
                    std::slice::from_ref(candidate_path),
                    &fields_to_resolve_locally,
                    cancellation_token,
                );

                match finding {
                    Ok((child_stack, child_leaves)) => {
                        if child_leaves.is_empty() && child_stack.is_empty() {
                            all_children_resolvable = false;
                            trace!(
                                "Path {} failed to resolve child '{}' locally.",
                                candidate_path.pretty_print(graph),
                                child_selection
                            );
                            break; // This candidate_path is invalid.
                        }
                    }
                    Err(_) => {
                        all_children_resolvable = false;
                        break; // This candidate_path is invalid.
                    }
                }
            }

            if all_children_resolvable {
                trace!(
                    "Path {} can resolve all children locally and is valid.",
                    candidate_path.pretty_print(graph)
                );
                valid_paths_for_children.push(candidate_path.clone());
            }
        }
        next_paths = valid_paths_for_children;
        if !field.is_leaf() && next_paths.is_empty() {
            // If no single path could satisfy all children, we have no valid way forward.
            return Err(WalkOperationError::NoPathsFound(field.name.to_string()));
        }
    }

    if field.is_leaf() {
        paths_per_leaf.push(find_best_paths(next_paths));
    } else {
        trace!("Found {} paths", next_paths.len());
        let next_paths = Rc::new(next_paths);
        for next_selection_items in &field.selections.items {
            next_stack_to_resolve.push((next_selection_items, Rc::clone(&next_paths)));
        }
    }

    Ok((next_stack_to_resolve, paths_per_leaf))
}

fn field_target_subgraph_ids(
    supergraph: &SupergraphState,
    field: &FieldSelection,
    paths: &[OperationPath<'_>],
    graph: &Graph,
) -> Result<Option<HashSet<String>>, WalkOperationError> {
    let mut parent_type_names = HashSet::new();

    for path in paths {
        let parent_node = graph.node(path.tail())?;
        parent_type_names.insert(parent_node.name_str());
    }

    let mut target_subgraph_ids = HashSet::new();

    for parent_type_name in parent_type_names {
        let Some(parent_def) = supergraph.definitions.get(parent_type_name) else {
            continue;
        };
        let Some(field_def) = parent_def.fields().get(&field.name) else {
            continue;
        };

        for graph_id in field_def.resolvable_in_graphs(parent_def) {
            if let Ok(subgraph_id) = supergraph.resolve_graph_id(&graph_id) {
                target_subgraph_ids.insert(subgraph_id.0);
            }
        }
    }

    if target_subgraph_ids.is_empty() {
        Ok(None)
    } else {
        Ok(Some(target_subgraph_ids))
    }
}
