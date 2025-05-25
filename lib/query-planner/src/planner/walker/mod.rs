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
        selection_set::{FieldSelection, InlineFragmentSelection},
    },
    graph::Graph,
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

#[instrument(skip(graph, fragment, paths), fields(
  type_condition = fragment.type_condition,
))]
fn process_inline_fragment<'a>(
    graph: &Graph,
    fragment: &'a InlineFragmentSelection,
    paths: &Vec<OperationPath>,
) -> Result<(ResolutionStack<'a>, Vec<Vec<OperationPath>>), WalkOperationError> {
    let mut stack_to_resolve: ResolutionStack = vec![];
    let mut paths_per_leaf: Vec<Vec<OperationPath>> = vec![];

    debug!(
        "Processing inline fragment '{}' on type '{}' through {} possible paths",
        fragment.type_condition,
        fragment.selections,
        paths.len()
    );

    for item in fragment.selections.items.iter() {
        let (next_stack_to_resolve, new_paths_per_leaf) = process_selection(graph, item, paths)?;
        paths_per_leaf.extend(new_paths_per_leaf);
        stack_to_resolve.extend(next_stack_to_resolve);
    }

    Ok((stack_to_resolve, paths_per_leaf))
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
        let direct_paths = find_direct_paths(graph, path, field, &excluded)?;

        debug!("Direct paths found: {}", direct_paths.len());

        if !direct_paths.is_empty() {
            advanced = true;

            for direct_path in direct_paths {
                tracker.add(&direct_path)?;
            }
        }

        let indirect_paths = find_indirect_paths(graph, path, field, &excluded)?;

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
