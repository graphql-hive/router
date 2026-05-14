mod apply_internal_aliases_patching;
mod batch_multi_type;
mod collapse_supertype_per_subgraph;
mod deduplicate_and_prune_fetch_steps;
mod merge_children_with_parents;
mod merge_leafs;
mod merge_passthrough_child;
mod merge_siblings;
mod turn_mutations_into_sequence;
mod type_mismatches;
mod utils;

use tracing::instrument;

use crate::{
    planner::fetch::{error::FetchGraphError, fetch_graph::FetchGraph, state::MultiTypeFetchStep},
    state::supergraph_state::SupergraphState,
    utils::cancellation::CancellationToken,
};

impl FetchGraph<MultiTypeFetchStep> {
    #[instrument(level = "trace", skip_all)]
    pub fn optimize(
        &mut self,
        supergraph_state: &SupergraphState,
        cancellation_token: &CancellationToken,
    ) -> Result<(), FetchGraphError> {
        // Run optimization passes repeatedly until the graph stabilizes, as one optimization can create
        // opportunities for others.
        loop {
            cancellation_token.bail_if_cancelled()?;
            let node_count_before = self.graph.node_count();
            let edge_count_before = self.graph.edge_count();

            self.merge_passthrough_child()?;
            self.merge_children_with_parents()?;
            self.merge_siblings()?;
            self.merge_leafs()?;
            self.deduplicate_and_prune_fetch_steps()?;
            self.batch_multi_type()?;

            // Per-subgraph supertype collapse rewrites selection maps in place without
            // touching node or edge counts, so we have to track its mutation signal
            // explicitly to keep iterating the loop when it opens new merge opportunities
            // for the structural passes above.
            let selections_mutated = self.collapse_supertype_per_subgraph(supergraph_state)?;

            let node_count_after = self.graph.node_count();
            let edge_count_after = self.graph.edge_count();

            if node_count_before == node_count_after
                && edge_count_before == edge_count_after
                && !selections_mutated
            {
                break;
            }
        }
        self.turn_mutations_into_sequence()?;
        self.fix_conflicting_type_mismatches(supergraph_state)?;

        // We call this last, because it should be done after all other optimizations/merging are done
        self.apply_internal_aliases_patching()?;

        Ok(())
    }
}
