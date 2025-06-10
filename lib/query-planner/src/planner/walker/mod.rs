mod best_path;
pub(crate) mod error;
mod excluded;
pub(crate) mod path;
pub(crate) mod pathfinder;
mod utils;

use crate::{
    ast::{
        operation::OperationDefinition,
        selection_item::SelectionItem,
        selection_set::{FieldSelection, InlineFragmentSelection, SelectionSet},
    },
    graph::{node::Node, Graph},
    planner::walker::pathfinder::NavigationTarget,
};
use best_path::{find_best_paths, BestPathTracker};
use error::WalkOperationError;
use excluded::ExcludedFromLookup;
use path::OperationPath;
use pathfinder::{find_direct_paths, find_indirect_paths};
use tracing::{debug, instrument, span, warn, Level};
use utils::get_entrypoints;

// TODO: Make a better struct
pub type BestPathsPerLeaf = Vec<Vec<OperationPath>>;

// TODO: Consider to use VecDeque(fixed_size) if we can predict it?
// TODO: Consider to drop this IR layer and just go with QTP directly.
type ResolutionStack<'a> = Vec<(&'a SelectionItem, Vec<OperationPath>)>;

#[instrument(skip(graph, operation))]
pub fn walk_operation(
    graph: &Graph,
    operation: &OperationDefinition,
) -> Result<BestPathsPerLeaf, WalkOperationError> {
    let (op_type, selection_set) = operation.parts();
    debug!("operation is of type {:?}", op_type);

    let root_entrypoints = get_entrypoints(graph, op_type)?;
    let initial_paths: Vec<OperationPath> = root_entrypoints
        .iter()
        .map(|edge| OperationPath::new_entrypoint(edge))
        .collect();

    let mut stack_to_resolve: ResolutionStack = vec![];

    for selection_item in selection_set.items.iter() {
        stack_to_resolve.push((selection_item, initial_paths.to_vec()));
    }

    let mut paths_per_leaf: Vec<Vec<OperationPath>> = vec![];

    while let Some((selection_item, paths)) = stack_to_resolve.pop() {
        let (next_stack_to_resolve, new_paths_per_leaf) =
            process_selection(graph, selection_item, &paths)?;

        paths_per_leaf.extend(new_paths_per_leaf);
        stack_to_resolve.extend(next_stack_to_resolve);
    }

    Ok(paths_per_leaf)
}

fn process_selection<'a>(
    graph: &Graph,
    selection_item: &'a SelectionItem,
    paths: &Vec<OperationPath>,
) -> Result<(ResolutionStack<'a>, Vec<Vec<OperationPath>>), WalkOperationError> {
    let mut stack_to_resolve: ResolutionStack = vec![];
    let mut paths_per_leaf: Vec<Vec<OperationPath>> = vec![];

    match selection_item {
        SelectionItem::InlineFragment(fragment) => {
            let (next_selection_items, new_paths_per_leaf) =
                process_inline_fragment(graph, fragment, paths)?;
            paths_per_leaf.extend(new_paths_per_leaf);
            stack_to_resolve.extend(next_selection_items);
        }
        SelectionItem::Field(field) => {
            let (next_selection_items, new_paths_per_leaf) = process_field(graph, field, paths)?;
            paths_per_leaf.extend(new_paths_per_leaf);
            stack_to_resolve.extend(next_selection_items);
        }
    }

    Ok((stack_to_resolve, paths_per_leaf))
}

#[instrument(skip_all)]
fn process_selection_set<'a>(
    graph: &Graph,
    selection_set: &'a SelectionSet,
    paths: &Vec<OperationPath>,
) -> Result<(ResolutionStack<'a>, Vec<Vec<OperationPath>>), WalkOperationError> {
    let mut stack_to_resolve: ResolutionStack = vec![];
    let mut paths_per_leaf: Vec<Vec<OperationPath>> = vec![];

    for item in selection_set.items.iter() {
        let (next_stack_to_resolve, new_paths_per_leaf) = process_selection(graph, item, paths)?;
        paths_per_leaf.extend(new_paths_per_leaf);
        stack_to_resolve.extend(next_stack_to_resolve);
    }

    Ok((stack_to_resolve, paths_per_leaf))
}

#[instrument(skip(graph, fragment, paths), fields(
  type_condition = fragment.type_condition,
))]
fn process_inline_fragment<'a>(
    graph: &Graph,
    fragment: &'a InlineFragmentSelection,
    paths: &Vec<OperationPath>,
) -> Result<(ResolutionStack<'a>, Vec<Vec<OperationPath>>), WalkOperationError> {
    debug!(
        "Processing inline fragment '{}' on type '{}' through {} possible paths",
        fragment.selections,
        fragment.type_condition,
        paths.len()
    );

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
        return process_selection_set(graph, &fragment.selections, paths);
    }

    debug!(
        "Trying to advance to: ... on {}, through {} possible paths",
        fragment.type_condition,
        paths.len()
    );

    let mut next_paths: Vec<OperationPath> = Vec::with_capacity(paths.len());
    for path in paths {
        let path_span = span!(Level::INFO, "explore_path", path = path.pretty_print(graph));
        let _enter = path_span.enter();

        let excluded = ExcludedFromLookup::new();
        let mut direct_paths = find_direct_paths(
            graph,
            path,
            &NavigationTarget::ConcreteType(&fragment.type_condition),
            &excluded,
        )?;

        debug!("Direct paths found: {}", direct_paths.len());

        if !direct_paths.is_empty() {
            debug!("advanced: {}", path.pretty_print(graph));
            next_paths.push(direct_paths.remove(0));
        } else {
            // Looks like a union member or an interface implementation is not resolvable.
            // The fact the fragment for that object type passed GraphQL validations,
            // means that it's a child of the abstract type,
            // and it was probably eliminated from the Graph because of intersection.
            warn!(
                "Object type '{}' is not resolvable by '{}', resolve only the __typename",
                fragment.type_condition, tail_type_name
            );
        }
    }

    if next_paths.is_empty() {
        let mut tracker = BestPathTracker::new(graph);

        for path in paths {
            let path_span = span!(Level::INFO, "explore_path", path = path.pretty_print(graph));
            let _enter = path_span.enter();
            let excluded = ExcludedFromLookup::new();
            let direct_paths = find_direct_paths(
                graph,
                path,
                &NavigationTarget::Field(&FieldSelection::new_typename()),
                &excluded,
            )?;

            debug!("Direct paths found: {}", direct_paths.len());

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

    process_selection_set(graph, &fragment.selections, &next_paths)
}

#[instrument(skip(graph, field, paths), fields(
  field_name = &field.name,
  leaf = field.is_leaf()
))]
fn process_field<'a>(
    graph: &Graph,
    field: &'a FieldSelection,
    paths: &Vec<OperationPath>,
) -> Result<(ResolutionStack<'a>, Vec<Vec<OperationPath>>), WalkOperationError> {
    let mut next_stack_to_resolve: ResolutionStack = vec![];
    let mut paths_per_leaf: Vec<Vec<OperationPath>> = vec![];
    let mut tracker = BestPathTracker::new(graph);

    debug!(
        "Trying to advance to: {} through {} possible paths",
        field,
        paths.len()
    );

    for path in paths {
        let path_span = span!(Level::INFO, "explore_path", path = path.pretty_print(graph));
        let _enter = path_span.enter();

        let mut advanced = false;

        let excluded = ExcludedFromLookup::new();
        let direct_paths =
            find_direct_paths(graph, path, &NavigationTarget::Field(field), &excluded)?;

        debug!("Direct paths found: {}", direct_paths.len());

        if !direct_paths.is_empty() {
            advanced = true;

            for direct_path in direct_paths {
                tracker.add(&direct_path)?;
            }
        }

        let indirect_paths =
            find_indirect_paths(graph, path, &NavigationTarget::Field(field), &excluded)?;

        debug!("Indirect paths found: {}", indirect_paths.len());

        if !indirect_paths.is_empty() {
            advanced = true;

            for indirect_path in indirect_paths {
                tracker.add(&indirect_path)?;
            }
        }

        match advanced {
            true => debug!("advanced: {}", path.pretty_print(graph)),
            false => warn!("failed to advance: {}", path.pretty_print(graph)),
        };
    }

    let next_paths = tracker.get_best_paths();

    if next_paths.is_empty() {
        return Err(WalkOperationError::NoPathsFound(field.name.to_string()));
    }

    if field.is_leaf() {
        paths_per_leaf.push(find_best_paths(next_paths));
    } else {
        debug!("Found {} paths", next_paths.len());

        for next_selection_items in field.selections.items.iter() {
            next_stack_to_resolve.push((next_selection_items, next_paths.clone()));
        }
    }

    Ok((next_stack_to_resolve, paths_per_leaf))
}
