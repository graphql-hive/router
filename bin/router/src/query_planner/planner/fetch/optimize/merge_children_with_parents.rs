use std::collections::VecDeque;

use petgraph::{graph::NodeIndex, Direction};
use tracing::{instrument, trace};

use crate::query_planner::planner::fetch::{
    error::FetchGraphError,
    fetch_graph::FetchGraph,
    optimize::utils::{perform_fetch_step_merge, MergedSteps},
    state::MultiTypeFetchStep,
};

impl FetchGraph<MultiTypeFetchStep> {
    #[instrument(level = "trace", skip_all)]
    pub(crate) fn merge_children_with_parents(&mut self) -> Result<(), FetchGraphError> {
        let root_index = self
            .root_index
            .ok_or(FetchGraphError::NonSingleRootStep(0))?;
        // Breadth-First Search (BFS) starting from the root node.
        let mut queue = VecDeque::from([root_index]);
        let mut merged_steps = MergedSteps::default();

        while let Some(parent_index) = queue.pop_front() {
            // Store pairs of sibling nodes that can be merged.
            let mut merges_to_perform: Vec<(NodeIndex, NodeIndex)> = Vec::new();
            let parent_index = merged_steps.resolve(parent_index);

            let children: Vec<_> = self
                .graph
                .neighbors_directed(parent_index, Direction::Outgoing)
                .collect();

            let parent = self.get_step_data(parent_index)?;

            for child_index in children.iter() {
                queue.push_back(*child_index);
                // Add the current child to the queue for further processing (BFS).
                let child = self.get_step_data(*child_index)?;

                if parent.can_merge(parent_index, *child_index, child, self) {
                    trace!(
                        "optimization found: merge parent [{}] with child [{}]",
                        parent_index.index(),
                        child_index.index()
                    );
                    merges_to_perform.push((parent_index, *child_index));
                }
            }

            for (parent_index, child_index) in merges_to_perform {
                let parent_index = merged_steps.resolve(parent_index);
                let child_index = merged_steps.resolve(child_index);
                // An earlier merge may have joined them already, or changed one of them.
                if parent_index == child_index {
                    continue;
                }
                let parent = self.get_step_data(parent_index)?;
                let child = self.get_step_data(child_index)?;
                if !parent.can_merge(parent_index, child_index, child, self) {
                    continue;
                }

                perform_fetch_step_merge(parent_index, child_index, self, false)?;
                merged_steps.record(child_index, parent_index);
            }
        }

        Ok(())
    }
}
