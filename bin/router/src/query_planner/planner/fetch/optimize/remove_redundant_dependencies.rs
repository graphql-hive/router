use petgraph::{graph::EdgeIndex, visit::EdgeRef};
use tracing::instrument;

use crate::query_planner::planner::fetch::{
    error::FetchGraphError, fetch_graph::FetchGraph,
    optimize::utils::is_reachable_via_alternative_upstream_path,
};

impl FetchGraph {
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
    pub(crate) fn remove_redundant_dependencies(&mut self) -> Result<(), FetchGraphError> {
        // Every step is made for something a parent has, and merges keep that. So there is no
        // step left without a parent, or without anything to fetch and nothing after it.
        debug_assert!(
            self.step_indices().all(|index| {
                self.root_index == Some(index)
                    || self.children_of(index).next().is_some()
                    || (self.parents_of(index).next().is_some()
                        && self
                            .get_step_data(index)
                            .is_ok_and(|step| !step.output.is_empty()))
            }),
            "a fetch step is left without a parent, or with nothing to fetch"
        );

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
