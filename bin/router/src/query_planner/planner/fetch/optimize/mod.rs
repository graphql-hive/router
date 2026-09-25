mod batch_multi_type;
mod fold_concrete_selections_to_interfaces;
mod merge_children_with_parents;
mod merge_leafs;
mod merge_passthrough_child;
mod merge_siblings;
mod normalize_selection_sets;
mod remove_redundant_dependencies;
mod turn_mutations_into_sequence;
mod type_mismatches;
mod utils;

use tracing::instrument;

use crate::query_planner::{
    planner::{
        fetch::{error::FetchGraphError, fetch_graph::FetchGraph},
        QueryPlannerOptions,
    },
    state::supergraph_state::SupergraphState,
    utils::cancellation::CancellationToken,
};

impl FetchGraph {
    #[instrument(level = "trace", skip_all)]
    pub fn optimize(
        &mut self,
        supergraph_state: &SupergraphState,
        options: &QueryPlannerOptions,
        cancellation_token: &CancellationToken,
    ) -> Result<(), FetchGraphError> {
        // Run optimization passes repeatedly until the graph stabilizes, as one optimization can create
        // opportunities for others.
        loop {
            cancellation_token.bail_if_cancelled()?;
            let node_count_before = self.graph.node_count();
            let edge_count_before = self.graph.edge_count();

            self.merge_passthrough_child()?;
            self.merge_children_with_parents(supergraph_state)?;
            self.merge_siblings(supergraph_state)?;
            self.merge_leafs(supergraph_state)?;
            self.remove_redundant_dependencies()?;
            self.batch_multi_type(supergraph_state)?;

            let node_count_after = self.graph.node_count();
            let edge_count_after = self.graph.edge_count();

            if node_count_before == node_count_after && edge_count_before == edge_count_after {
                break;
            }
        }

        // The rest only shapes each fetch's operation, it doesn't move selections between
        // fetches, so it runs once, after the merges settled.
        self.normalize_selection_sets(supergraph_state)?;
        self.fold_concrete_selections_to_interfaces(supergraph_state, options)?;
        self.turn_mutations_into_sequence()?;
        if cfg!(debug_assertions) {
            self.validate_operations(supergraph_state)?;
        }
        self.fix_conflicting_type_mismatches(supergraph_state)?;

        Ok(())
    }
}
