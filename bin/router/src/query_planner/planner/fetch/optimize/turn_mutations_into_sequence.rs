use std::collections::{BTreeMap, HashSet};

use petgraph::{
    graph::NodeIndex,
    visit::{Bfs, EdgeRef},
};
use tracing::instrument;

use crate::query_planner::{
    planner::fetch::{error::FetchGraphError, fetch_graph::FetchGraph, state::MultiTypeFetchStep},
    state::supergraph_state::OperationKind,
};

impl FetchGraph<MultiTypeFetchStep> {
    /// Root mutation fields run one after another, and a field is done only when every fetch it
    /// needs is done, entity calls included. So the steps of field N+1 wait for the last steps of
    /// field N, not for the root.
    ///
    /// Runs before any optimization. With the order in the graph, merges keep it like any other
    /// dependency, and nothing else has to know about mutations.
    #[instrument(level = "trace", skip_all)]
    pub(crate) fn turn_mutations_into_sequence(&mut self) -> Result<(), FetchGraphError> {
        if self.operation_kind != OperationKind::Mutation {
            return Ok(());
        }

        let root_index = self
            .root_index
            .ok_or(FetchGraphError::NonSingleRootStep(0))?;

        let mut fields = BTreeMap::<usize, Vec<NodeIndex>>::new();
        for edge_ref in self.children_of(root_index) {
            let step_index = edge_ref.target();
            let position = self
                .get_step_data(step_index)?
                .mutation_field_position
                .ok_or(FetchGraphError::MutationStepWithNoOrder)?;
            fields.entry(position).or_default().push(step_index);
        }

        let fields: Vec<Vec<NodeIndex>> = fields.into_values().collect();
        for pair in fields.windows(2) {
            let (previous, next) = (&pair[0], &pair[1]);
            let next_steps = self.reachable_from(next);
            let last_steps: Vec<NodeIndex> = self
                .reachable_from(previous)
                .into_iter()
                .filter(|step| self.children_of(*step).next().is_none())
                // A step both fields use can't wait for the second one.
                .filter(|step| !next_steps.contains(step))
                .collect();

            for next_step in next {
                if let Some(edge) = self.graph.find_edge(root_index, *next_step) {
                    self.remove_edge(edge);
                }
                for last_step in &last_steps {
                    self.connect(*last_step, *next_step);
                }
            }
        }

        Ok(())
    }

    fn reachable_from(&self, starts: &[NodeIndex]) -> HashSet<NodeIndex> {
        let mut reachable = HashSet::new();
        for start in starts {
            let mut bfs = Bfs::new(&self.graph, *start);
            while let Some(step) = bfs.next(&self.graph) {
                reachable.insert(step);
            }
        }
        reachable
    }
}
