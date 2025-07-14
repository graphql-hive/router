use petgraph::graph::NodeIndex;
use tracing::{instrument, trace};

use crate::{
    ast::type_aware_selection::find_arguments_conflicts,
    planner::fetch::{
        error::FetchGraphError, fetch_graph::FetchGraph, fetch_step_data::FetchStepData,
        optimize::utils::perform_fetch_step_merge,
    },
};

impl FetchStepData {
    pub fn can_merge_leafs(
        &self,
        self_index: NodeIndex,
        other_index: NodeIndex,
        other: &Self,
        fetch_graph: &FetchGraph,
    ) -> bool {
        // `other` must be a leaf node (no children).
        if fetch_graph.children_of(other_index).count() != 0 {
            return false;
        }

        if self_index == other_index {
            return false;
        }

        if self.service_name != other.service_name {
            return false;
        }

        if self.response_path != other.response_path {
            return false;
        }

        if self.input.type_name != other.input.type_name {
            return false;
        }

        if self.condition != other.condition {
            return false;
        }

        // otherwise we break the order of mutations
        if self.mutation_field_position != other.mutation_field_position {
            return false;
        }

        let input_conflicts = find_arguments_conflicts(&self.input, &other.input);

        if !input_conflicts.is_empty() {
            return false;
        }

        true
    }
}

impl FetchGraph {
    #[instrument(level = "trace", skip_all)]
    /// This optimization is about merging leaf nodes in the fetch nodes with other nodes.
    /// It reduces the number of fetch steps, without degrading the query performance.
    /// The query performance is not degraded, because the leaf node has no children,
    /// meaning the overall depth (amount of parallel layers) is not increased.
    pub(crate) fn merge_leafs(&mut self) -> Result<(), FetchGraphError> {
        while let Some((target_idx, leaf_idx)) = self.find_merge_candidate()? {
            trace!(
                "optimization found: merge leaf [{}] with [{}]",
                leaf_idx.index(),
                target_idx.index(),
            );
            perform_fetch_step_merge(target_idx, leaf_idx, self)?;
        }

        Ok(())
    }

    fn find_merge_candidate(&self) -> Result<Option<(NodeIndex, NodeIndex)>, FetchGraphError> {
        let leafs: Vec<NodeIndex> = self
            .graph
            .node_indices()
            .filter(|&idx| self.children_of(idx).count() == 0 && self.root_index != Some(idx))
            .collect();

        for i in 0..leafs.len() {
            for j in (i + 1)..leafs.len() {
                let target_idx = leafs[i];
                let leaf_idx = leafs[j];

                let target_data = self.get_step_data(target_idx)?;
                let leaf_data = self.get_step_data(leaf_idx)?;

                if target_data.can_merge_leafs(target_idx, leaf_idx, leaf_data, self) {
                    return Ok(Some((target_idx, leaf_idx)));
                }
            }
        }

        Ok(None)
    }
}
