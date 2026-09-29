use std::collections::{HashMap, VecDeque};

use petgraph::{graph::NodeIndex, Direction};
use tracing::{instrument, trace};

use crate::query_planner::planner::fetch::{
    error::FetchGraphError, fetch_graph::FetchGraph, fetch_step_data::FetchStepData,
    optimize::utils::perform_fetch_step_merge, state::MultiTypeFetchStep,
};

impl FetchGraph<MultiTypeFetchStep> {
    #[instrument(level = "trace", skip_all)]
    pub(crate) fn merge_siblings(&mut self) -> Result<(), FetchGraphError> {
        let root_index = self
            .root_index
            .ok_or(FetchGraphError::NonSingleRootStep(0))?;
        // Breadth-First Search (BFS) starting from the root node.
        let mut queue = VecDeque::from([root_index]);

        while let Some(parent_index) = queue.pop_front() {
            // Store pairs of sibling nodes that can be merged.
            // The additional Vec<usize> is an indicator for conflicting field indexes in the 2nd sibling.
            // If the Vec is empty, it means there are no conflicts.
            let mut merges_to_perform = Vec::<(NodeIndex, NodeIndex)>::new();

            // HashMap to keep track of node index mappings, especially after merges.
            // Key: original index, Value: potentially updated index after merges.
            let mut node_indexes: HashMap<NodeIndex, NodeIndex> = HashMap::new();

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
                        // Register their original indexes in the map.
                        node_indexes.insert(*sibling_index, *sibling_index);
                        node_indexes.insert(*other_sibling_index, *other_sibling_index);

                        merges_to_perform.push((*sibling_index, *other_sibling_index));

                        // Since a merge is possible, move to the next child to avoid redundant checks.
                        break;
                    }
                }
            }

            for (child_index, other_child_index) in merges_to_perform {
                // Get the latest indexes for the nodes, accounting for previous merges.
                let child_index_latest = node_indexes
                    .get(&child_index)
                    .ok_or(FetchGraphError::IndexMappingLost)?;
                let other_child_index_latest = node_indexes
                    .get(&other_child_index)
                    .ok_or(FetchGraphError::IndexMappingLost)?;

                perform_fetch_step_merge(
                    *child_index_latest,
                    *other_child_index_latest,
                    self,
                    false,
                )?;

                // Because `other_child` was merged into `child`,
                // then everything that was pointing to `other_child`
                // has to point to the `child`.
                node_indexes.insert(*other_child_index_latest, *child_index_latest);
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
