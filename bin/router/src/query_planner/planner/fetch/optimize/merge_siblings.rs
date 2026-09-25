use std::collections::VecDeque;

use petgraph::{graph::NodeIndex, Direction};
use tracing::{instrument, trace};

use crate::query_planner::planner::fetch::{
    error::FetchGraphError, fetch_graph::FetchGraph, fetch_step_data::FetchStepData,
    optimize::utils::try_merge_steps,
};
use crate::query_planner::state::supergraph_state::SupergraphState;

impl FetchGraph {
    #[instrument(level = "trace", skip_all)]
    pub(crate) fn merge_siblings(
        &mut self,
        supergraph: &SupergraphState,
    ) -> Result<(), FetchGraphError> {
        let root_index = self
            .root_index
            .ok_or(FetchGraphError::NonSingleRootStep(0))?;
        // Breadth-First Search (BFS) starting from the root node.
        let mut queue = VecDeque::from([root_index]);

        while let Some(parent_index) = queue.pop_front() {
            if !self.graph.contains_node(parent_index) {
                continue;
            }

            // Sort fetch steps by mutation's field position,
            // to execute mutations in correct order.
            let mut siblings_with_pos: Vec<(NodeIndex, Option<usize>)> = self
                .graph
                .neighbors_directed(parent_index, Direction::Outgoing)
                .map(|sibling| {
                    self.get_step_data(sibling)
                        .map(|data| (sibling, data.mutation_field_position))
                })
                .collect::<Result<_, _>>()?;
            siblings_with_pos.sort_by_key(|(_node, pos)| *pos);

            let siblings: Vec<NodeIndex> =
                siblings_with_pos.into_iter().map(|(idx, _)| idx).collect();

            for (i, sibling_index) in siblings.iter().enumerate() {
                // A sibling merged into an earlier one is gone.
                if !self.graph.contains_node(*sibling_index) {
                    continue;
                }
                // Add the current node to the queue for further processing (BFS).
                queue.push_back(*sibling_index);

                for other_sibling_index in siblings.iter().skip(i + 1) {
                    if !self.graph.contains_node(*other_sibling_index) {
                        continue;
                    }

                    // Checked against the sibling as it is now, with whatever it took in
                    // already.
                    let current = self.get_step_data(*sibling_index)?;
                    let other_sibling = self.get_step_data(*other_sibling_index)?;
                    if current.can_merge_siblings(
                        *sibling_index,
                        *other_sibling_index,
                        other_sibling,
                        self,
                    ) && try_merge_steps(
                        *sibling_index,
                        *other_sibling_index,
                        self,
                        false,
                        supergraph,
                    )? {
                        trace!(
                            "Found siblings optimization: {} <- {}",
                            sibling_index.index(),
                            other_sibling_index.index()
                        );
                    }
                }
            }
        }
        Ok(())
    }
}

impl FetchStepData {
    pub(crate) fn can_merge_siblings(
        &self,
        self_index: NodeIndex,
        other_index: NodeIndex,
        other: &Self,
        fetch_graph: &FetchGraph,
    ) -> bool {
        if let (Some(self_mut_idx), Some(other_mut_index)) =
            (self.mutation_field_position, other.mutation_field_position)
        {
            // If indexes are equal or one happens to be after the other,
            // and we already know they belong to the same service,
            // we shouldn't prevent merging.
            if self_mut_idx != other_mut_index
                && (self_mut_idx as i64 - other_mut_index as i64).abs() != 1
            {
                return false;
            }
        }

        if fetch_graph.is_ancestor_or_descendant(self_index, other_index) {
            // Looks like they depend on each other
            return false;
        }

        self.can_merge(self_index, other_index, other, fetch_graph)
    }
}
