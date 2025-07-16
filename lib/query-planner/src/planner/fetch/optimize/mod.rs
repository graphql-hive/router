mod apply_internal_aliases_patching;
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
    planner::fetch::{error::FetchGraphError, fetch_graph::FetchGraph},
    state::supergraph_state::SupergraphState,
};

impl FetchGraph {
    #[instrument(level = "trace", skip_all)]
    pub fn optimize(&mut self, supergraph_state: &SupergraphState) -> Result<(), FetchGraphError> {
        // Run optimization passes repeatedly until the graph stabilizes, as one optimization can create
        // opportunities for others.
        loop {
            let node_count_before = self.graph.node_count();
            let edge_count_before = self.graph.edge_count();

            self.merge_passthrough_child()?;
            self.merge_children_with_parents()?;
            self.merge_siblings()?;
            self.merge_leafs()?;
            self.deduplicate_and_prune_fetch_steps()?;

            let node_count_after = self.graph.node_count();
            let edge_count_after = self.graph.edge_count();

            if node_count_before == node_count_after && edge_count_before == edge_count_after {
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
