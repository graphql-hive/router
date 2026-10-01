use std::collections::{btree_map::Entry, BTreeMap, HashMap, HashSet};

use petgraph::graph::EdgeIndex;

use crate::query_planner::graph::Graph;

use super::{error::WalkOperationError, path::OperationPath};

pub struct BestPathTracker<'graph> {
    graph: &'graph Graph,
    /// A map from subgraph name to the best path and its cost.
    /// BTreeMap instead of HashMap to keep the order of inserted keys deterministic.
    subgraph_to_best_paths: BTreeMap<&'graph str, (Vec<OperationPath<'graph>>, u64)>,
}

pub fn find_best_paths<'graph>(paths: Vec<OperationPath<'graph>>) -> Vec<OperationPath<'graph>> {
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

/// One path per requirement leaf. Any of a leaf's tied best paths reaches it, so keeping them
/// all fetches the same value more than once. Picking one per leaf on its own can still send
/// the leaves down different routes, like `whiskers` through `Animal`'s key and `tricks`
/// through `Dog`'s. So each leaf takes the path whose edges the most leaves could use too,
/// the first one on a tie.
pub fn pick_shared_paths<'graph>(
    tied_paths_per_leaf: Vec<Vec<OperationPath<'graph>>>,
) -> impl Iterator<Item = OperationPath<'graph>> {
    // Nothing to pick from when no leaf has a tie, and that's almost always.
    let mut leaves_per_edge: HashMap<EdgeIndex, usize> = HashMap::new();
    if tied_paths_per_leaf.iter().any(|paths| paths.len() > 1) {
        for paths in &tied_paths_per_leaf {
            let edges: HashSet<EdgeIndex> =
                paths.iter().flat_map(|path| path.get_edges()).collect();
            for edge in edges {
                *leaves_per_edge.entry(edge).or_default() += 1;
            }
        }
    }

    tied_paths_per_leaf.into_iter().filter_map(move |paths| {
        if paths.len() == 1 {
            return paths.into_iter().next();
        }
        // `rev` so that `max_by_key`, which keeps the last maximum, keeps the first.
        paths.into_iter().rev().max_by_key(|path| {
            path.get_edges()
                .iter()
                .map(|edge| leaves_per_edge[edge])
                .sum::<usize>()
        })
    })
}

impl<'graph> BestPathTracker<'graph> {
    pub fn new(graph: &'graph Graph) -> Self {
        Self {
            graph,
            subgraph_to_best_paths: BTreeMap::new(),
        }
    }

    pub fn add(&mut self, path: &OperationPath<'graph>) -> Result<(), WalkOperationError> {
        let tail_graph_id = self
            .graph
            .node(path.tail())?
            .graph_id()
            .expect("Graph ID not found in node");

        match self.subgraph_to_best_paths.entry(tail_graph_id) {
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

    pub fn get_best_paths(self) -> Vec<OperationPath<'graph>> {
        self.subgraph_to_best_paths
            .into_values()
            .flat_map(|(paths, _)| paths)
            .collect::<Vec<OperationPath<'graph>>>()
    }
}
