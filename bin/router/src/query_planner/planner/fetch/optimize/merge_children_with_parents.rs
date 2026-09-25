use std::collections::{HashMap, VecDeque};

use petgraph::{graph::NodeIndex, Direction};
use tracing::{instrument, trace};

use crate::query_planner::planner::fetch::{
    error::FetchGraphError, fetch_graph::FetchGraph, optimize::utils::try_merge_steps,
};
use crate::query_planner::state::supergraph_state::SupergraphState;

impl FetchGraph {
    #[instrument(level = "trace", skip_all)]
    pub(crate) fn merge_children_with_parents(
        &mut self,
        supergraph: &SupergraphState,
    ) -> Result<(), FetchGraphError> {
        let root_index = self
            .root_index
            .ok_or(FetchGraphError::NonSingleRootStep(0))?;
        // Breadth-First Search (BFS) starting from the root node.
        let mut queue = VecDeque::from([root_index]);
        // A child merged into its parent is the parent from then on.
        let mut merged_into: HashMap<NodeIndex, NodeIndex> = HashMap::new();

        while let Some(mut parent_index) = queue.pop_front() {
            while let Some(merged) = merged_into.get(&parent_index) {
                parent_index = *merged;
            }

            let children: Vec<_> = self
                .graph
                .neighbors_directed(parent_index, Direction::Outgoing)
                .collect();

            for child_index in children {
                queue.push_back(child_index);

                // Each merge changes the parent, so the next child is checked against the
                // parent as it is now.
                let parent = self.get_step_data(parent_index)?;
                let child = self.get_step_data(child_index)?;
                if parent.can_merge(parent_index, child_index, child, self)
                    && try_merge_steps(parent_index, child_index, self, false, supergraph)?
                {
                    trace!(
                        "optimization found: merged child [{}] into parent [{}]",
                        child_index.index(),
                        parent_index.index()
                    );
                    merged_into.insert(child_index, parent_index);
                }
            }
        }

        Ok(())
    }
}
