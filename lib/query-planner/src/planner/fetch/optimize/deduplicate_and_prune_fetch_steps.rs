use petgraph::{
    graph::{EdgeIndex, NodeIndex},
    visit::EdgeRef,
};
use tracing::{instrument, trace};

use crate::planner::fetch::{
    error::FetchGraphError, fetch_graph::FetchGraph,
    optimize::utils::is_reachable_via_alternative_upstream_path, state::MultiTypeFetchStep,
};

impl FetchGraph<MultiTypeFetchStep> {
    /// Removes redundant direct dependencies from a FetchStep graph.
    ///
    /// ```text
    /// in:
    /// A -> C
    /// A -> B -> ... -> C
    /// out:
    /// A -> B -> ... -> C
    /// ```
    #[instrument(level = "trace", skip_all)]
    pub(crate) fn deduplicate_and_prune_fetch_steps(&mut self) -> Result<(), FetchGraphError> {
        let steps_to_remove: Vec<NodeIndex> = self
            .step_indices()
            .filter(|&step_index| {
                let step = match self.get_step_data(step_index) {
                    Ok(s) => s,
                    Err(_) => return false,
                };

                if !step.output.is_empty() && self.parents_of(step_index).next().is_some() {
                    return false;
                }

                if self.children_of(step_index).next().is_some() {
                    return false;
                }

                trace!("optimization found: remove '{}'", step);

                true
            })
            .collect();

        for step_index in steps_to_remove {
            self.remove_step(step_index);
        }

        let mut edges_to_remove: Vec<EdgeIndex> = vec![];
        for step_index in self.step_indices() {
            for parent_to_step_edge in self.parents_of(step_index) {
                let direct_parent_index = parent_to_step_edge.source();
                let child_index = step_index;
                if is_reachable_via_alternative_upstream_path(
                    self,
                    child_index,
                    direct_parent_index,
                )? {
                    edges_to_remove.push(parent_to_step_edge.id());
                }
            }
        }

        for edge_index in edges_to_remove {
            self.remove_edge(edge_index);
        }

        Ok(())
    }
}
