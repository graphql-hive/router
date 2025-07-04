use std::collections::{HashMap, VecDeque};

use petgraph::{graph::NodeIndex, Direction};
use tracing::{instrument, trace};

use crate::planner::fetch::{
    error::FetchGraphError, fetch_graph::FetchGraph, optimize::utils::perform_fetch_step_merge,
};

impl FetchGraph {
    #[instrument(level = "trace", skip_all)]
    pub(crate) fn merge_children_with_parents(&mut self) -> Result<(), FetchGraphError> {
        let root_index = self
            .root_index
            .ok_or(FetchGraphError::NonSingleRootStep(0))?;
        // Breadth-First Search (BFS) starting from the root node.
        let mut queue = VecDeque::from([root_index]);
        // HashMap to keep track of node index mappings, especially after merges.
        // Key: original index, Value: potentially updated index after merges.
        let mut node_indexes: HashMap<NodeIndex, NodeIndex> = HashMap::new();

        node_indexes.insert(root_index, root_index);

        while let Some(parent_index) = queue.pop_front() {
            // Store pairs of sibling nodes that can be merged.
            let mut merges_to_perform: Vec<(NodeIndex, NodeIndex)> = Vec::new();
            let parent_index = *node_indexes
                .get(&parent_index)
                .expect("Index mapping got lost");

            let children: Vec<_> = self
                .graph
                .neighbors_directed(parent_index, Direction::Outgoing)
                .collect();

            let parent = self.get_step_data(parent_index)?;

            for child_index in children.iter() {
                queue.push_back(*child_index);
                // Add the current child to the queue for further processing (BFS).
                let child = self.get_step_data(*child_index)?;
                node_indexes.insert(*child_index, *child_index);
                node_indexes.insert(parent_index, parent_index);

                if parent.can_merge(parent_index, *child_index, child, self) {
                    trace!(
                        "optimization found: merge parent [{}] with child [{}]",
                        parent_index.index(),
                        child_index.index()
                    );
                    // Register their original indexes in the map.
                    merges_to_perform.push((parent_index, *child_index));
                }
            }

            for (parent_index, child_index) in merges_to_perform {
                // Get the latest indexes for the nodes, accounting for previous merges.
                let parent_index_latest = node_indexes
                    .get(&parent_index)
                    .expect("Index mapping got lost");
                let child_index_latest = node_indexes
                    .get(&child_index)
                    .expect("Index mapping got lost");

                perform_fetch_step_merge(*parent_index_latest, *child_index_latest, self)?;

                // Because `child` was merged into `parent`,
                // then everything that was pointing to `child`
                // has to point to the `parent`.
                node_indexes.insert(*child_index_latest, *parent_index_latest);
            }
        }

        Ok(())
    }
}
