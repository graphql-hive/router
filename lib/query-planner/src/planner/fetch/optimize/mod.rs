mod apply_internal_aliases_patching;
mod deduplicate_and_prune_fetch_steps;
mod merge_children_with_parents;
mod merge_passthrough_child;
mod merge_siblings;
mod turn_mutations_into_sequence;
mod utils;

use tracing::instrument;

use crate::planner::fetch::{error::FetchGraphError, fetch_graph::FetchGraph};

impl FetchGraph {
    #[instrument(level = "trace", skip_all)]
    pub fn optimize(&mut self) -> Result<(), FetchGraphError> {
        self.merge_passthrough_child()?;
        self.merge_children_with_parents()?;
        self.merge_siblings()?;
        self.deduplicate_and_prune_fetch_steps()?;
        self.turn_mutations_into_sequence()?;

        // We call this last, because it should be done after all other optimizations/merging are done
        self.apply_internal_aliases_patching()?;

        Ok(())
    }
}
