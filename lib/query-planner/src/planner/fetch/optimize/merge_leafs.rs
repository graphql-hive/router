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

        // `other` must be a leaf node (no children).
        if fetch_graph.children_of(other_index).count() != 0 {
            return false;
        }

        // We can't merge if one is a descendant of the other,
        // because there's a dependency between them,
        // that could lead to incorrect results.
        // Either input of "other" depends on the output of "self",
        // or input of "other" depends on the output of one of the steps in between.
        if fetch_graph.is_descendant_of(other_index, self_index) {
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
            perform_fetch_step_merge(target_idx, leaf_idx, self)?;
        }

        Ok(())
    }

    fn find_merge_candidate(&self) -> Result<Option<(NodeIndex, NodeIndex)>, FetchGraphError> {
        let all_nodes: Vec<NodeIndex> = self
            .graph
            .node_indices()
            .filter(|&idx| self.root_index != Some(idx))
            .collect();

        for &target_idx in &all_nodes {
            for &leaf_idx in &all_nodes {
                let target_data = self.get_step_data(target_idx)?;
                let leaf_data = self.get_step_data(leaf_idx)?;

                if target_data.can_merge_leafs(target_idx, leaf_idx, leaf_data, self) {
                    trace!(
                        "optimization found: merge leaf [{}] with [{}]",
                        leaf_idx.index(),
                        target_idx.index(),
                    );
                    return Ok(Some((target_idx, leaf_idx)));
                }
            }
        }

        Ok(None)
    }
}
