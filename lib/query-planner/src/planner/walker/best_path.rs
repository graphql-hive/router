use std::collections::{hash_map::Entry, HashMap};

use crate::graph::Graph;

use super::{error::WalkOperationError, path::OperationPath};

pub struct BestPathTracker<'graph> {
    graph: &'graph Graph,
    /// A map from subgraph name to the best path and its cost.
    subgraph_to_best_paths: HashMap<String, (Vec<OperationPath>, u64)>,
}

pub fn find_best_paths(paths: Vec<OperationPath>) -> Vec<OperationPath> {
    let mut best_paths = Vec::new();
    let mut best_cost = 0;

    for path in paths {
        if best_cost == 0 {
            best_cost = path.cost;
            best_paths = vec![path];
        } else if best_cost == path.cost {
            best_paths.push(path);
        } else if best_cost > path.cost {
            best_cost = path.cost;
            best_paths = vec![path];
        }
    }

    best_paths
}

impl<'graph> BestPathTracker<'graph> {
    pub fn new(graph: &'graph Graph) -> Self {
        Self {
            graph,
            subgraph_to_best_paths: HashMap::new(),
        }
    }

    pub fn add(&mut self, path: &OperationPath) -> Result<(), WalkOperationError> {
        let tail_graph_id = self
            .graph
            .node(path.tail())?
            .graph_id()
            .expect("Graph ID not found in node");

        match self.subgraph_to_best_paths.entry(tail_graph_id.to_string()) {
            Entry::Occupied(mut entry) => {
                let (existing_paths, existing_cost) = entry.get_mut();

                match path.cost.cmp(existing_cost) {
                    std::cmp::Ordering::Less => {
                        *existing_cost = path.cost;
                        existing_paths.clear();
                        existing_paths.push(path.clone());
                    }
                    std::cmp::Ordering::Equal => {
                        existing_paths.push(path.clone());
                    }
                    std::cmp::Ordering::Greater => {
                        // ignore this path
                    }
                }
            }
            Entry::Vacant(entry) => {
                entry.insert((vec![path.clone()], path.cost));
            }
        }

        Ok(())
    }

    pub fn get_best_paths(self) -> Vec<OperationPath> {
        self.subgraph_to_best_paths
            .into_values()
            .flat_map(|(paths, _)| paths)
            .collect::<Vec<OperationPath>>()
    }
}
