use bumpalo::{collections::Vec as BumpVec, Bump};

use super::{error::WalkOperationError, path::OperationPath, WalkContext};

// A tuple representing the best paths for a specific subgraph.
// (subgraph_id, paths, cost)
type SubgraphBestPaths<'bump> = (&'bump str, BumpVec<'bump, OperationPath<'bump>>, u64);

pub struct BestPathTracker<'bump, 'a> {
    ctx: &'a WalkContext<'bump>,
    /// A vector of best paths per subgraph. We use a Vec instead of a map
    /// for better performance with the bump allocator.
    subgraph_to_best_paths: BumpVec<'bump, SubgraphBestPaths<'bump>>,
}

pub fn find_best_paths<'bump>(
    arena: &'bump Bump,
    paths: BumpVec<'bump, OperationPath<'bump>>,
) -> BumpVec<'bump, OperationPath<'bump>> {
    let mut best_paths = BumpVec::new_in(arena);
    let mut best_cost = u64::MAX;

    for path in paths {
        if best_cost == u64::MAX || path.cost < best_cost {
            best_cost = path.cost;
            // The old `best_paths` are now suboptimal, clear and start a new list.
            if !best_paths.is_empty() {
                best_paths.clear();
            }
            best_paths.push(path);
        } else if path.cost == best_cost {
            best_paths.push(path);
        }
    }

    best_paths
}

impl<'bump, 'a> BestPathTracker<'bump, 'a> {
    pub fn new(ctx: &'a WalkContext<'bump>) -> Self {
        Self {
            ctx,
            subgraph_to_best_paths: BumpVec::new_in(ctx.arena),
        }
    }

    pub fn add(&mut self, path: &OperationPath<'bump>) -> Result<(), WalkOperationError> {
        let tail_graph_id = self
            .ctx
            .graph
            .node(path.tail())?
            .graph_id()
            .expect("Graph ID not found in node");

        if let Some((_, existing_paths, existing_cost)) = self
            .subgraph_to_best_paths
            .iter_mut()
            .find(|(id, _, _)| *id == tail_graph_id)
        {
            // Found existing entry for this subgraph
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
                    // ignore this path, it's worse
                }
            }
        } else {
            // No entry for this subgraph yet, create a new one.
            let mut new_paths = BumpVec::new_in(self.ctx.arena);
            new_paths.push(path.clone());
            // Allocate the graph_id string in the arena
            let graph_id_in_arena = self.ctx.arena.alloc_str(tail_graph_id);
            self.subgraph_to_best_paths
                .push((graph_id_in_arena, new_paths, path.cost));
        }

        Ok(())
    }

    pub fn get_best_paths(mut self) -> BumpVec<'bump, OperationPath<'bump>> {
        // Sorting here to maintain the deterministic behavior of the old BTreeMap.
        self.subgraph_to_best_paths.sort_by_key(|(id, _, _)| *id);

        let mut result = BumpVec::new_in(self.ctx.arena);
        for (_, paths, _) in self.subgraph_to_best_paths {
            result.extend(paths);
        }
        result
    }
}
