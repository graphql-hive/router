use std::collections::HashSet;

use petgraph::visit::{EdgeRef, NodeRef};
use tracing::{debug, instrument};

use crate::{
    graph::{
        edge::{Edge, EdgeReference},
        selection::{Selection, SelectionNode, SelectionNodeField},
        Graph,
    },
    planner::{
        tree::query_tree_node::QueryTreeNode,
        walker::best_path::{find_best_paths, BestPathTracker},
    },
};

use super::{error::WalkOperationError, excluded::ExcludedFromLookup, path::OperationPath};

pub type VisitedGraphs = HashSet<String>;

struct IndirectPathsLookupQueue {
    queue: Vec<(VisitedGraphs, HashSet<Selection>, OperationPath)>,
}

impl IndirectPathsLookupQueue {
    pub fn new_from_excluded(excluded: &ExcludedFromLookup, path: &OperationPath) -> Self {
        IndirectPathsLookupQueue {
            queue: vec![(
                excluded.graph_ids.clone(),
                excluded
                    .requirement
                    .clone()
                    .into_iter()
                    .collect::<HashSet<_>>(),
                path.clone(),
            )],
        }
    }

    pub fn add(
        &mut self,
        visited_graphs: VisitedGraphs,
        selections: HashSet<Selection>,
        path: OperationPath,
    ) {
        self.queue.push((visited_graphs, selections, path));
    }

    pub fn pop(&mut self) -> Option<(VisitedGraphs, HashSet<Selection>, OperationPath)> {
        self.queue.pop()
    }
}

#[instrument(skip(graph, excluded), ret(), fields(
  path = path.pretty_print(graph),
  current_cost = path.cost
))]
pub fn find_indirect_paths(
    graph: &Graph,
    path: &OperationPath,
    field_name: &str,
    excluded: &ExcludedFromLookup,
) -> Result<Vec<OperationPath>, WalkOperationError> {
    let mut tracker = BestPathTracker::new(graph);
    let tail_node_index = path.tail();
    let tail_node = graph.node(tail_node_index)?;
    let source_graph_id = tail_node
        .graph_id()
        .ok_or(WalkOperationError::TailMissingInfo(tail_node_index))?;

    let mut queue = IndirectPathsLookupQueue::new_from_excluded(excluded, path);

    while let Some(item) = queue.pop() {
        let (visited_graphs, visited_key_fields, path) = item;

        let relevant_edges = graph
            .edges_from(path.tail())
            .filter(|e| matches!(e.weight(), Edge::EntityMove { .. }));

        for edge_ref in relevant_edges {
            debug!(
                "Exploring edge {}",
                graph.pretty_print_edge(edge_ref.id(), false)
            );

            let edge_tail_graph_id = graph.node(edge_ref.target().id())?.graph_id().unwrap();

            if visited_graphs.contains(edge_tail_graph_id) {
                debug!(
                    "Ignoring, graph is excluded and already visited (current: {}, visited: {:?})",
                    edge_tail_graph_id, visited_graphs
                );
                continue;
            }

            let edge_tail_graph_id = graph.node(edge_ref.target().id())?.graph_id().unwrap();

            if edge_tail_graph_id == source_graph_id {
                // Prevent a situation where we are going back to the same graph
                // The only exception is when we are moving to an abstract type
                debug!("Ignoring. We would go back to the same graph");
                continue;
            }

            // A huge win for performance, is when you do less work :D
            // We can ignore an edge that has already been visited with the same key fields / requirements.
            // The way entity-move edges are created, where every graph points to every other graph:
            //  Graph A: User @key(id) @key(name)
            //  Graph B: User @key(id)
            //  Edges in a merged graph:
            //    - User/A @key(id) -> User/B
            //    - User/B @key(id) -> User/A
            //    - User/B @key(name) -> User/A
            // Allows us to ignore an edge with the same key fields.
            // That's because in some other path, we will or already have checked the other edge.
            let edge = edge_ref.weight();

            let requirements_already_checked = match edge.requirements_selections() {
                Some(selection_requirements) => visited_key_fields.contains(selection_requirements),
                None => false,
            };

            if requirements_already_checked {
                debug!("Ignoring. Already visited similar edge");
                continue;
            }

            let new_excluded =
                excluded.next(edge_tail_graph_id, &visited_key_fields, &[edge_ref.id()]);

            let can_be_satisfied = can_satisfy_edge(graph, &edge_ref, &path, &new_excluded, false)?;

            match can_be_satisfied {
                None => {
                    debug!("Requirements not satisfied, continue look up...");
                    continue;
                }
                Some(paths) => {
                    debug!(
                        "Advancing path to {}",
                        graph.pretty_print_edge(edge_ref.id(), false)
                    );

                    let next_resolution_path =
                        path.advance(&edge_ref, QueryTreeNode::from_paths(graph, &paths)?);

                    let direct_paths_excluded =
                        excluded.next(edge_tail_graph_id, &visited_key_fields, &[]);
                    let direct_paths = find_direct_paths(
                        graph,
                        &next_resolution_path,
                        field_name,
                        &direct_paths_excluded,
                    )?;

                    if !direct_paths.is_empty() {
                        debug!(
                            "Found {} direct paths to {}",
                            direct_paths.len(),
                            graph.pretty_print_edge(edge_ref.id(), false)
                        );

                        for direct_path in direct_paths {
                            tracker.add(&direct_path)?;
                        }

                        continue;
                    } else {
                        debug!("No direct paths found");

                        let mut new_visited_graphs = visited_graphs.clone();
                        new_visited_graphs.insert(edge_tail_graph_id.to_string());

                        let next_requirements = match edge.requirements_selections() {
                            Some(requirements) => {
                                let mut new_visited_key_fields = visited_key_fields.clone();
                                new_visited_key_fields.insert(requirements.clone());
                                new_visited_key_fields
                            }
                            None => visited_key_fields.clone(),
                        };

                        queue.add(new_visited_graphs, next_requirements, next_resolution_path);

                        debug!("going deeper");
                    }
                }
            }
        }
    }

    let best_paths = tracker.get_best_paths();

    debug!(
        "Finished finding indirect paths, found total of {}",
        best_paths.len()
    );

    // TODO: this should be done in a more efficient way, like I do in the satisfiability checker
    // I set shortest path right after each path is generated

    Ok(best_paths)
}

#[instrument(skip(graph, excluded), ret(), fields(
    path = path.pretty_print(graph),
    current_cost = path.cost
))]
pub fn find_direct_paths(
    graph: &Graph,
    path: &OperationPath,
    field_name: &str,
    excluded: &ExcludedFromLookup,
) -> Result<Vec<OperationPath>, WalkOperationError> {
    let mut result: Vec<OperationPath> = vec![];
    let path_tail_index = path.tail();

    // Get all the edges from the current tail
    // Filter by FieldMove edges with matching field name and not already in path, to avoid loops
    let edges_iter =
            graph
            .edges_from(path_tail_index)
            .filter(|e| matches!(e.weight(), Edge::FieldMove { name, .. } if name == field_name && !path.has_visited_edge(&e.id())));

    for edge_ref in edges_iter {
        debug!(
            "checking edge {}",
            graph.pretty_print_edge(edge_ref.id(), false)
        );

        let node = graph.node(edge_ref.target())?;
        let new_excluded = excluded.next_with_graph_id(node.graph_id().unwrap());
        let can_be_satisfied = can_satisfy_edge(graph, &edge_ref, path, &new_excluded, false)?;

        match can_be_satisfied {
            Some(paths) => {
                debug!(
                    "Advancing path {} with edge {}",
                    path.pretty_print(graph),
                    graph.pretty_print_edge(edge_ref.id(), false)
                );

                let next_resolution_path =
                    path.advance(&edge_ref, QueryTreeNode::from_paths(graph, &paths)?);

                result.push(next_resolution_path);
            }
            None => {
                debug!("Edge not satisfied, continue look up...");
            }
        }
    }

    Ok(result)
}

#[instrument(skip_all, ret(), fields(
  path = path.pretty_print(graph),
  edge = edge_ref.weight().display_name(),
))]
fn can_satisfy_edge(
    graph: &Graph,
    edge_ref: &EdgeReference,
    path: &OperationPath,
    excluded: &ExcludedFromLookup,
    use_only_direct_edges: bool,
) -> Result<Option<Vec<OperationPath>>, WalkOperationError> {
    let edge = edge_ref.weight();

    match edge.requirements_selections() {
        None => Ok(Some(vec![])),
        Some(selections) => {
            debug!(
                "checking requirements {} for edge '{}'",
                selections,
                graph.pretty_print_edge(edge_ref.id(), false)
            );

            let mut requirements: Vec<MoveRequirement> = vec![];
            let mut paths_to_requirements: Vec<OperationPath> = vec![];

            for selection in selections.selection_set.iter() {
                requirements.splice(
                    0..0,
                    vec![MoveRequirement {
                        paths: vec![path.clone()],
                        selection: selection.clone(),
                    }],
                );
            }

            // it's important to pop from the end as we want to process the last added requirement first
            while let Some(requirement) = requirements.pop() {
                match &requirement.selection {
                    SelectionNode::Field(selection_field_requirement) => {
                        let result = validate_field_requirement(
                            graph,
                            &requirement,
                            selection_field_requirement,
                            excluded,
                            use_only_direct_edges,
                        )?;

                        match result {
                            Some((next_paths, next_requirements)) => {
                                debug!("Paths for {}", selection_field_requirement);

                                for next_path in next_paths.iter() {
                                    debug!("  Path {} is valid", next_path.pretty_print(graph));
                                }

                                if selection_field_requirement.is_leaf() {
                                    let best_paths = find_best_paths(next_paths);
                                    debug!(
                                        "Found {} best paths for this leaf requirement",
                                        best_paths.len()
                                    );

                                    for best_path in best_paths {
                                        paths_to_requirements.push(
                                            path.build_requirement_continuation_path(&best_path),
                                        );
                                    }
                                }

                                requirements.splice(0..0, next_requirements);
                            }
                            None => {
                                return Ok(None);
                            }
                        };
                    }
                    SelectionNode::Fragment { .. } => {
                        unimplemented!("fragment not supported yet")
                    }
                }
            }

            for path in paths_to_requirements.iter() {
                debug!("path {} is valid", path.pretty_print(graph));
            }

            Ok(Some(paths_to_requirements))
        }
    }
}

#[derive(Debug)]
pub struct MoveRequirement {
    pub paths: Vec<OperationPath>,
    pub selection: SelectionNode,
}

type FieldRequirementsResult = Option<(Vec<OperationPath>, Vec<MoveRequirement>)>;

#[instrument(skip_all, ret())]
fn validate_field_requirement(
    graph: &Graph,
    move_requirement: &MoveRequirement,
    field_move_requirement: &SelectionNodeField,
    excluded: &ExcludedFromLookup,
    use_only_direct_edges: bool,
) -> Result<FieldRequirementsResult, WalkOperationError> {
    let field_name = &field_move_requirement.field_name;
    let mut next_paths: Vec<OperationPath> = Vec::new();

    for path in move_requirement.paths.iter() {
        let direct_paths = find_direct_paths(graph, path, field_name, excluded)?;

        for direct_path in direct_paths.into_iter() {
            next_paths.push(direct_path);
        }
    }

    if !use_only_direct_edges {
        for path in move_requirement.paths.iter() {
            let indirect_paths = find_indirect_paths(graph, path, field_name, excluded)?;

            for indirect_path in indirect_paths.into_iter() {
                next_paths.push(indirect_path);
            }
        }
    }

    if next_paths.is_empty() {
        return Ok(None);
    }

    if move_requirement.selection.selections().is_none()
        || move_requirement
            .selection
            .selections()
            .is_some_and(|s| s.is_empty())
    {
        return Ok(Some((next_paths, vec![])));
    }

    let next_requirements: Vec<MoveRequirement> = move_requirement
        .selection
        .selections()
        .unwrap()
        .iter()
        .map(|selection| MoveRequirement {
            selection: selection.clone(),
            paths: next_paths.clone(),
        })
        .collect();

    Ok(Some((next_paths, next_requirements)))
}
