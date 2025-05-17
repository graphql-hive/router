mod best_path;
mod excluded;
mod pathfinder;
mod utils;

pub mod error;
pub mod path;

use crate::{ast::selection_item::SelectionItem, graph::Graph};
use best_path::{find_best_paths, BestPathTracker};
use error::WalkOperationError;
use excluded::ExcludedFromLookup;
use graphql_parser_hive_fork::query::OperationDefinition;
use path::OperationPath;
use pathfinder::{find_direct_paths, find_indirect_paths};
use tracing::{debug, instrument, span, warn, Level};
use utils::{get_entrypoints, operation_to_parts};

// TODO: Make a better struct
type BestPathsPerLeaf = Vec<Vec<OperationPath>>;

// TODO: Consider to use VecDeque(fixed_size) if we can predict it?
// TODO: Consider to drop this IR layer and just go with QTP directly.
type ResolutionStack<'a> = Vec<(&'a SelectionItem, Vec<OperationPath>)>;

#[instrument(skip(graph, operation))]
pub fn walk_operation(
    graph: &Graph,
    operation: &OperationDefinition<'static, String>,
) -> Result<BestPathsPerLeaf, WalkOperationError> {
    let (op_type, selection_set) = operation_to_parts(operation);
    debug!("operation is of type {:?}", op_type);

    let root_entrypoints = get_entrypoints(graph, &op_type)?;
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
        let mut tracker = BestPathTracker::new(graph);

        match selection_item {
            SelectionItem::Fragment(_f) => unimplemented!("fragments are not supported yet"),
            SelectionItem::Field(field) => {
                let field_span = span!(
                    Level::INFO,
                    "process_selection",
                    field = &field.name,
                    leaf = field.is_leaf()
                );
                let _enter = field_span.enter();

                debug!(
                    "Trying to advance to: {} through {} possible paths",
                    selection_item,
                    paths.len()
                );

                for path in paths {
                    let path_span =
                        span!(Level::INFO, "explore_path", path = path.pretty_print(graph),);

                    let _enter2 = path_span.enter();
                    let mut advanced = false;

                    let excluded = ExcludedFromLookup::new();
                    let direct_paths = find_direct_paths(graph, &path, &field.name, &excluded)?;

                    debug!("Direct paths found: {}", direct_paths.len());

                    if !direct_paths.is_empty() {
                        advanced = true;

                        for direct_path in direct_paths {
                            tracker.add(&direct_path)?;
                        }
                    }

                    let indirect_paths = find_indirect_paths(graph, &path, &field.name, &excluded)?;

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
                    return Err(WalkOperationError::NoPathsFound(selection_item.clone()));
                }

                if field.is_leaf() {
                    paths_per_leaf.push(find_best_paths(next_paths));
                } else {
                    debug!("Found {} paths", next_paths.len());

                    for next_selection_items in field.selections.items.iter() {
                        stack_to_resolve.push((next_selection_items, next_paths.clone()));
                    }
                }
            }
        }
    }

    Ok(paths_per_leaf)
}
