use petgraph::{
    graph::{EdgeIndex, NodeIndex},
    visit::{EdgeRef, NodeRef},
};
use tracing::instrument;

use crate::planner::fetch::{
    error::FetchGraphError, fetch_graph::FetchGraph, state::MultiTypeFetchStep,
};

impl FetchGraph<MultiTypeFetchStep> {
    #[instrument(level = "trace", skip_all)]
    pub(crate) fn turn_mutations_into_sequence(&mut self) -> Result<(), FetchGraphError> {
        let root_index = self
            .root_index
            .ok_or(FetchGraphError::NonSingleRootStep(0))?;

        if !is_mutation_fetch_step(self, root_index)? {
            return Ok(());
        }

        let mut node_mutation_field_pos_pairs: Vec<(NodeIndex, usize)> = Vec::new();
        let mut edge_ids_to_remove: Vec<EdgeIndex> = Vec::new();

        for edge_ref in self.children_of(root_index) {
            edge_ids_to_remove.push(edge_ref.id());
            let node_index = edge_ref.target().id();
            let mutation_field_pos = self
                .get_step_data(node_index)?
                .mutation_field_position
                .ok_or(FetchGraphError::MutationStepWithNoOrder)?;
            node_mutation_field_pos_pairs.push((node_index, mutation_field_pos));
        }

        node_mutation_field_pos_pairs.sort_by_key(|&(_, pos)| pos);

        let mut new_edges_pairs: Vec<(NodeIndex, NodeIndex)> = Vec::new();
        let mut iter = node_mutation_field_pos_pairs.iter();
        let mut current = iter.next();

        for next_sequence_child in iter {
            if let Some((current_node_index, _pos)) = current {
                let next_node_index = next_sequence_child.0;
                new_edges_pairs.push((current_node_index.id(), next_node_index));
            }
            current = Some(next_sequence_child);
        }

        for edge_id in edge_ids_to_remove {
            self.remove_edge(edge_id);
        }

        // Bring back the root -> Mutation edge
        let first_pair = node_mutation_field_pos_pairs
            .first()
            .ok_or(FetchGraphError::EmptyFetchSteps)?;
        self.connect(root_index, first_pair.0);

        for (from_id, to_id) in new_edges_pairs {
            self.connect(from_id, to_id);
        }

        Ok(())
    }
}

fn is_mutation_fetch_step(
    fetch_graph: &FetchGraph<MultiTypeFetchStep>,
    fetch_step_index: NodeIndex,
) -> Result<bool, FetchGraphError> {
    for edge_ref in fetch_graph.children_of(fetch_step_index) {
        let child = fetch_graph.get_step_data(edge_ref.target().id())?;

        if !child.output.is_selecting_definition("Mutation") {
            return Ok(false);
        }
    }

    Ok(true)
}
