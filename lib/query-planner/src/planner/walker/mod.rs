mod best_path;
pub(crate) mod error;
mod excluded;
pub(crate) mod path;
pub(crate) mod pathfinder;
mod utils;

use std::collections::VecDeque;

use crate::{
    ast::{
        operation::OperationDefinition,
        selection_item::SelectionItem,
        selection_set::{FieldSelection, InlineFragmentSelection, SelectionSet},
    },
    graph::{node::Node, Graph},
    planner::walker::pathfinder::NavigationTarget,
    state::supergraph_state::OperationKind,
};
use best_path::{find_best_paths, BestPathTracker};
use bumpalo::{collections::Vec as BumpVec, Bump};
use error::WalkOperationError;
use excluded::ExcludedFromLookup;
use path::OperationPath;
use pathfinder::{find_direct_paths, find_indirect_paths};
use tracing::{instrument, span, trace, Level};
use utils::get_entrypoints;

/// The context for a single walk operation, holding the arena allocator.
pub struct WalkContext<'bump> {
    pub arena: &'bump Bump,
    pub graph: &'bump Graph,
}

impl<'bump> WalkContext<'bump> {
    pub fn new(graph: &'bump Graph, arena: &'bump Bump) -> Self {
        Self { arena, graph }
    }
}

/// The result of a successful walk, containing the operation kind and the groups of paths for each root field.
pub struct ResolvedOperation<'a> {
    pub operation_kind: OperationKind,
    pub root_field_groups: BumpVec<'a, BestPathsPerLeaf<'a>>,
}

// TODO: Make a better struct
pub type BestPathsPerLeaf<'a> = BumpVec<'a, BumpVec<'a, OperationPath<'a>>>;

// TODO: Consider to use VecDeque(fixed_size) if we can predict it?
// TODO: Consider to drop this IR layer and just go with QTP directly.
type WorkItem<'a> = (&'a SelectionItem, BumpVec<'a, OperationPath<'a>>);
type ResolutionStack<'a> = BumpVec<'a, WorkItem<'a>>;

#[instrument(level = "trace", skip(ctx, operation))]
pub fn walk_operation<'a>(
    ctx: &'a WalkContext<'a>,
    operation: &'a OperationDefinition,
) -> Result<ResolvedOperation<'a>, WalkOperationError> {
    let operation_kind = operation
        .operation_kind
        .clone()
        .unwrap_or(OperationKind::Query);
    let (op_type, selection_set) = operation.parts();
    trace!("operation is of type {:?}", op_type);

    let root_entrypoints = get_entrypoints(ctx.graph, op_type)?;
    let mut initial_paths = BumpVec::new_in(ctx.arena);
    initial_paths.extend(
        root_entrypoints
            .iter()
            .map(|edge| OperationPath::new_entrypoint(ctx, edge)),
    );

    let mut paths_grouped_by_root_field =
        BumpVec::with_capacity_in(operation.selection_set.items.len(), ctx.arena);

    // It's critical to iterate over root fiels and preserve their original order
    for selection_item in selection_set.items.iter() {
        let mut stack_to_resolve: VecDeque<WorkItem<'a>> = VecDeque::new();

        stack_to_resolve.push_back((selection_item, initial_paths.clone()));

        let mut paths_per_leaf = BumpVec::new_in(ctx.arena);

        while let Some((selection_item, paths)) = stack_to_resolve.pop_front() {
            let (next_stack_to_resolve, new_paths_per_leaf) =
                process_selection(ctx, selection_item, &paths)?;

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
    ctx: &'a WalkContext<'a>,
    selection_item: &'a SelectionItem,
    paths: &BumpVec<'a, OperationPath<'a>>,
) -> Result<(ResolutionStack<'a>, BestPathsPerLeaf<'a>), WalkOperationError> {
    let mut stack_to_resolve = BumpVec::new_in(ctx.arena);
    let mut paths_per_leaf = BumpVec::new_in(ctx.arena);

    match selection_item {
        SelectionItem::InlineFragment(fragment) => {
            let (next_selection_items, new_paths_per_leaf) =
                process_inline_fragment(ctx, fragment, paths)?;
            paths_per_leaf.extend(new_paths_per_leaf);
            stack_to_resolve.extend(next_selection_items);
        }
        SelectionItem::Field(field) => {
            let (next_selection_items, new_paths_per_leaf) = process_field(ctx, field, paths)?;
            paths_per_leaf.extend(new_paths_per_leaf);
            stack_to_resolve.extend(next_selection_items);
        }
        SelectionItem::FragmentSpread(_) => {
            // No processing needed for FragmentSpread
        }
    }

    Ok((stack_to_resolve, paths_per_leaf))
}

#[instrument(level = "trace", skip(ctx, selection_set, paths))]
fn process_selection_set<'a>(
    ctx: &'a WalkContext<'a>,
    selection_set: &'a SelectionSet,
    paths: &BumpVec<'a, OperationPath<'a>>,
) -> Result<(ResolutionStack<'a>, BestPathsPerLeaf<'a>), WalkOperationError> {
    let mut stack_to_resolve = BumpVec::new_in(ctx.arena);
    let mut paths_per_leaf = BumpVec::new_in(ctx.arena);

    for item in selection_set.items.iter() {
        let (next_stack_to_resolve, new_paths_per_leaf) = process_selection(ctx, item, paths)?;
        paths_per_leaf.extend(new_paths_per_leaf);
        stack_to_resolve.extend(next_stack_to_resolve);
    }

    Ok((stack_to_resolve, paths_per_leaf))
}

#[instrument(level = "trace",skip(ctx, fragment, paths), fields(
  type_condition = fragment.type_condition,
))]
fn process_inline_fragment<'a>(
    ctx: &'a WalkContext<'a>,
    fragment: &'a InlineFragmentSelection,
    paths: &BumpVec<'a, OperationPath<'a>>,
) -> Result<(ResolutionStack<'a>, BestPathsPerLeaf<'a>), WalkOperationError> {
    trace!(
        "Processing inline fragment '{}' on type '{}' through {} possible paths",
        fragment.selections,
        fragment.type_condition,
        paths.len()
    );

    // if the current type is an object type we ignore an abstract move
    // but if it's a union, we need to find an abstract move to the target type
    let tail_index = ctx.graph.get_edge_tail(
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
    let tail = ctx.graph.node(tail_index)?;
    let tail_type_name = match tail {
        Node::SubgraphType(t) => &t.name,
        _ => panic!("Expected a subgraph type when resolving fragments"),
    };

    if tail_type_name == &fragment.type_condition {
        return process_selection_set(ctx, &fragment.selections, paths);
    }

    trace!(
        "Trying to advance to: ... on {}, through {} possible paths",
        fragment.type_condition,
        paths.len()
    );

    let mut next_paths = BumpVec::with_capacity_in(paths.len(), ctx.arena);
    for path in paths {
        let path_span = span!(
            Level::TRACE,
            "explore_path",
            path = path.pretty_print(ctx.graph)
        );
        let _enter = path_span.enter();

        let mut direct_paths = find_direct_paths(
            ctx,
            path,
            &NavigationTarget::ConcreteType(&fragment.type_condition),
        )?;

        trace!("Direct paths found: {}", direct_paths.len());
        if !direct_paths.is_empty() {
            trace!("advanced: {}", path.pretty_print(ctx.graph));
            next_paths.push(direct_paths.remove(0));
        }

        let mut indirect_paths = find_indirect_paths(
            ctx,
            path,
            &NavigationTarget::ConcreteType(&fragment.type_condition),
            &ExcludedFromLookup::new(),
        )?;

        if !indirect_paths.is_empty() {
            trace!("advanced: {}", path.pretty_print(ctx.graph));
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

    if next_paths.is_empty() {
        let mut tracker = BestPathTracker::new(ctx);

        for path in paths {
            let path_span = span!(
                Level::TRACE,
                "explore_path",
                path = path.pretty_print(ctx.graph)
            );
            let _enter = path_span.enter();
            let direct_paths = find_direct_paths(
                ctx,
                path,
                &NavigationTarget::Field(&FieldSelection::new_typename()),
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

        let next_paths_from_tracker = tracker.get_best_paths();

        if next_paths_from_tracker.is_empty() {
            return Err(WalkOperationError::NoPathsFound("__typename".to_string()));
        }

        let best = find_best_paths(ctx.arena, next_paths_from_tracker);
        let mut result = BumpVec::new_in(ctx.arena);
        result.push(best);
        return Ok((ResolutionStack::new_in(ctx.arena), result));
    }

    process_selection_set(ctx, &fragment.selections, &next_paths)
}

#[instrument(level = "trace", skip(ctx, field, paths), fields(
  field_name = &field.name,
  leaf = field.is_leaf()
))]
fn process_field<'a>(
    ctx: &'a WalkContext<'a>,
    field: &'a FieldSelection,
    paths: &BumpVec<'a, OperationPath<'a>>,
) -> Result<(ResolutionStack<'a>, BestPathsPerLeaf<'a>), WalkOperationError> {
    let mut next_stack_to_resolve = BumpVec::new_in(ctx.arena);
    let mut paths_per_leaf = BumpVec::new_in(ctx.arena);
    let mut tracker = BestPathTracker::new(ctx);

    trace!(
        "Trying to advance to: {} through {} possible paths",
        field,
        paths.len()
    );

    for path in paths {
        let path_span = span!(
            Level::TRACE,
            "explore_path",
            path = path.pretty_print(ctx.graph)
        );
        let _enter = path_span.enter();

        let mut advanced = false;

        let excluded = ExcludedFromLookup::new();
        let direct_paths = find_direct_paths(ctx, path, &NavigationTarget::Field(field))?;
        trace!("Direct paths found: {}", direct_paths.len());

        if !direct_paths.is_empty() {
            advanced = true;
            for direct_path in direct_paths {
                tracker.add(&direct_path)?;
            }
        }

        let indirect_paths =
            find_indirect_paths(ctx, path, &NavigationTarget::Field(field), &excluded)?;
        trace!("Indirect paths found: {}", indirect_paths.len());

        if !indirect_paths.is_empty() {
            advanced = true;
            for indirect_path in indirect_paths {
                tracker.add(&indirect_path)?;
            }
        }

        trace!(
            "{}: {}",
            if advanced {
                "advanced"
            } else {
                "failed to advance"
            },
            path.pretty_print(ctx.graph)
        );
    }

    let next_paths = tracker.get_best_paths();
    if next_paths.is_empty() {
        return Err(WalkOperationError::NoPathsFound(field.name.to_string()));
    }

    if field.is_leaf() {
        paths_per_leaf.push(find_best_paths(ctx.arena, next_paths));
    } else {
        trace!("Found {} paths", next_paths.len());
        for next_selection_items in &field.selections.items {
            next_stack_to_resolve.push((next_selection_items, next_paths.clone()));
        }
    }
    Ok((next_stack_to_resolve, paths_per_leaf))
}
