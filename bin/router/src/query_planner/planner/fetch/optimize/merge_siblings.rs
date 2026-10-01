use std::collections::VecDeque;

use petgraph::{graph::NodeIndex, Direction};
use tracing::{instrument, trace};

use crate::query_planner::planner::fetch::{
    error::FetchGraphError,
    fetch_graph::FetchGraph,
    fetch_step_data::FetchStepData,
    optimize::utils::{perform_fetch_step_merge, MergedSteps},
    state::MultiTypeFetchStep,
};

impl FetchGraph<MultiTypeFetchStep> {
    #[instrument(level = "trace", skip_all)]
    pub(crate) fn merge_siblings(&mut self) -> Result<(), FetchGraphError> {
        let root_index = self
            .root_index
            .ok_or(FetchGraphError::NonSingleRootStep(0))?;
        // Breadth-First Search (BFS) starting from the root node.
        let mut queue = VecDeque::from([root_index]);
        let mut merged_steps = MergedSteps::default();

        while let Some(parent_index) = queue.pop_front() {
            let parent_index = merged_steps.resolve(parent_index);
            // Store pairs of sibling nodes that can be merged.
            // The additional Vec<usize> is an indicator for conflicting field indexes in the 2nd sibling.
            // If the Vec is empty, it means there are no conflicts.
            let mut merges_to_perform = Vec::<(NodeIndex, NodeIndex)>::new();

            let siblings: Vec<NodeIndex> = self
                .graph
                .neighbors_directed(parent_index, Direction::Outgoing)
                .collect();

            for (i, sibling_index) in siblings.iter().enumerate() {
                // Add the current node to the queue for further processing (BFS).
                queue.push_back(*sibling_index);
                let current = self.get_step_data(*sibling_index)?;

                // Iterate through the remaining children (siblings) to check for merge possibilities.
                for other_sibling_index in siblings.iter().skip(i + 1) {
                    let other_sibling = self.get_step_data(*other_sibling_index)?;

                    trace!(
                        "checking if [{}] and [{}] can be merged",
                        sibling_index.index(),
                        other_sibling_index.index()
                    );

                    if current.can_merge_siblings(
                        *sibling_index,
                        *other_sibling_index,
                        other_sibling,
                        self,
                    ) {
                        trace!(
                            "Found siblings optimization: {} <- {}",
                            sibling_index.index(),
                            other_sibling_index.index()
                        );
                        merges_to_perform.push((*sibling_index, *other_sibling_index));

                        // Since a merge is possible, move to the next child to avoid redundant checks.
                        break;
                    }
                }
            }

            for (child_index, other_child_index) in merges_to_perform {
                let child_index = merged_steps.resolve(child_index);
                let other_child_index = merged_steps.resolve(other_child_index);
                // An earlier merge may have joined them already, or changed one of them.
                if child_index == other_child_index {
                    continue;
                }
                let child = self.get_step_data(child_index)?;
                let other_child = self.get_step_data(other_child_index)?;
                if !child.can_merge_siblings(child_index, other_child_index, other_child, self) {
                    continue;
                }

                perform_fetch_step_merge(child_index, other_child_index, self, false)?;
                merged_steps.record(other_child_index, child_index);
            }
        }
        Ok(())
    }
}

impl FetchStepData<MultiTypeFetchStep> {
    pub(crate) fn can_merge_siblings(
        &self,
        self_index: NodeIndex,
        other_index: NodeIndex,
        other: &Self,
        fetch_graph: &FetchGraph<MultiTypeFetchStep>,
    ) -> bool {
        // First, check if the base conditions for merging are met.
        let can_merge_base = self.can_merge(self_index, other_index, other, fetch_graph);

        if fetch_graph.is_ancestor_or_descendant(self_index, other_index) {
            // Looks like they depend on each other
            return false;
        }

        can_merge_base
    }
}
