mod best_path;
pub(crate) mod error;
mod excluded;
pub(crate) mod path;
pub(crate) mod pathfinder;
mod utils;

use std::collections::VecDeque;

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
use pathfinder::{find_direct_paths, find_indirect_paths};
use tracing::{instrument, span, trace, Level};
use utils::get_entrypoints;

pub struct ResolvedOperation {
    pub operation_kind: OperationKind,
    pub root_field_groups: Vec<BestPathsPerLeaf>,
}

// TODO: Make a better struct
pub type BestPathsPerLeaf = Vec<Vec<OperationPath>>;

// TODO: Consider to use VecDeque(fixed_size) if we can predict it?
// TODO: Consider to drop this IR layer and just go with QTP directly.

type WorkItem<'a> = (&'a SelectionItem, Vec<OperationPath>);
type ResolutionStack<'a> = Vec<WorkItem<'a>>;

#[instrument(level = "trace", skip_all)]
pub fn walk_operation(
    graph: &Graph,
    supergraph: &SupergraphState,
    override_context: &PlannerOverrideContext,
    operation: &OperationDefinition,
    cancellation_token: &CancellationToken,
) -> Result<ResolvedOperation, WalkOperationError> {
    let operation_kind = operation
        .operation_kind
        .clone()
        .unwrap_or(OperationKind::Query);
    let (op_type, selection_set) = operation.parts();
    trace!("operation is of type {:?}", op_type);

    let root_entrypoints = get_entrypoints(graph, op_type)?;
    let initial_paths: Vec<OperationPath> = root_entrypoints
        .iter()
        .map(|edge| OperationPath::new_entrypoint(edge))
        .collect();

    let mut paths_grouped_by_root_field: Vec<BestPathsPerLeaf> =
        Vec::with_capacity(operation.selection_set.items.len());

    // It's critical to iterate over root fiels and preserve their original order
    for selection_item in selection_set.items.iter() {
        let mut stack_to_resolve: VecDeque<WorkItem> = VecDeque::new();

        stack_to_resolve.push_back((selection_item, initial_paths.to_vec()));

        let mut paths_per_leaf: Vec<Vec<OperationPath>> = vec![];

        while let Some((selection_item, paths)) = stack_to_resolve.pop_front() {
            cancellation_token.bail_if_cancelled()?;
            let (next_stack_to_resolve, new_paths_per_leaf) = process_selection(
                graph,
                supergraph,
                override_context,
                selection_item,
                &paths,
                &vec![],
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

fn process_selection<'a>(
    graph: &'a Graph,
    supergraph: &'a SupergraphState,
    override_context: &'a PlannerOverrideContext,
    selection_item: &'a SelectionItem,
    paths: &Vec<OperationPath>,
    fields_to_resolve_locally: &Vec<String>,
    cancellation_token: &CancellationToken,
) -> Result<(ResolutionStack<'a>, Vec<Vec<OperationPath>>), WalkOperationError> {
    let mut stack_to_resolve: ResolutionStack = vec![];
    let mut paths_per_leaf: Vec<Vec<OperationPath>> = vec![];

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
fn process_selection_set<'a>(
    graph: &'a Graph,
    supergraph: &'a SupergraphState,
    override_context: &'a PlannerOverrideContext,
    selection_set: &'a SelectionSet,
    paths: &Vec<OperationPath>,
    fields_to_resolve_locally: &Vec<String>,
    cancellation_token: &CancellationToken,
) -> Result<(ResolutionStack<'a>, Vec<Vec<OperationPath>>), WalkOperationError> {
    let mut stack_to_resolve: ResolutionStack = vec![];
    let mut paths_per_leaf: Vec<Vec<OperationPath>> = vec![];

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
fn process_inline_fragment<'a>(
    graph: &'a Graph,
    supergraph: &'a SupergraphState,
    override_context: &'a PlannerOverrideContext,
    fragment: &'a InlineFragmentSelection,
    paths: &Vec<OperationPath>,
    fields_to_resolve_locally: &Vec<String>,
    cancellation_token: &CancellationToken,
) -> Result<(ResolutionStack<'a>, Vec<Vec<OperationPath>>), WalkOperationError> {
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

        let mut next_paths: Vec<OperationPath> = Vec::with_capacity(paths.len());
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

    let mut next_paths: Vec<OperationPath> = Vec::with_capacity(paths.len());
    for path in paths {
        let path_span = span!(
            Level::TRACE,
            "explore_path",
            path = path.pretty_print(graph)
        );
        let _enter = path_span.enter();

        let mut direct_paths = find_direct_paths(
            graph,
            override_context,
            path,
            &NavigationTarget::ConcreteType(&fragment.type_condition, fragment.into()),
            cancellation_token,
        )?;

        trace!("Direct paths found: {}", direct_paths.len());
        if !direct_paths.is_empty() {
            trace!("advanced: {}", path.pretty_print(graph));
            next_paths.push(direct_paths.remove(0));
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

            if indirect_paths.is_empty() && direct_paths.is_empty() {
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
                &NavigationTarget::Field(&FieldSelection::new_typename()),
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

#[instrument(level = "trace", skip_all, fields(
  field_name = &field.name,
  leaf = field.is_leaf()
))]
fn process_field<'a>(
    graph: &'a Graph,
    supergraph: &'a SupergraphState,
    override_context: &'a PlannerOverrideContext,
    field: &'a FieldSelection,
    paths: &[OperationPath],
    fields_to_resolve_locally: &[String],
    cancellation_token: &CancellationToken,
) -> Result<(ResolutionStack<'a>, Vec<Vec<OperationPath>>), WalkOperationError> {
    let mut next_stack_to_resolve: ResolutionStack = vec![];
    let mut paths_per_leaf: Vec<Vec<OperationPath>> = vec![];
    let mut tracker = BestPathTracker::new(graph);

    trace!(
        "Trying to advance to: {} through {} possible paths",
        field,
        paths.len()
    );

    cancellation_token.bail_if_cancelled()?;

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
            &NavigationTarget::Field(field),
            cancellation_token,
        )?;
        trace!("Direct paths found: {}", direct_paths.len());

        if !direct_paths.is_empty() {
            advanced = true;
            for direct_path in direct_paths {
                tracker.add(&direct_path)?;
            }
        }

        if !fields_to_resolve_locally.contains(&field.name) {
            let indirect_paths = find_indirect_paths(
                graph,
                override_context,
                path,
                &NavigationTarget::Field(field),
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
        let mut valid_paths_for_children: Vec<OperationPath> = Vec::with_capacity(next_paths.len());
        for candidate_path in &next_paths {
            let mut all_children_resolvable = true;
            // We don't need the results of the sub-walk here, only whether it was successful
            for child_selection in &field.selections.items {
                let finding = process_selection(
                    graph,
                    supergraph,
                    override_context,
                    child_selection,
                    &vec![candidate_path.clone()],
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
        for next_selection_items in &field.selections.items {
            next_stack_to_resolve.push((next_selection_items, next_paths.clone()));
        }
    }

    Ok((next_stack_to_resolve, paths_per_leaf))
}
