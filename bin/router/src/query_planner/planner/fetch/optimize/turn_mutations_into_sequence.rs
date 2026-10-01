use std::collections::HashSet;

use petgraph::{
    graph::{EdgeIndex, NodeIndex},
    visit::{Bfs, EdgeRef, NodeRef},
};
use tracing::instrument;

use crate::query_planner::{
    planner::fetch::{error::FetchGraphError, fetch_graph::FetchGraph, state::MultiTypeFetchStep},
    state::supergraph_state::OperationKind,
};

impl FetchGraph<MultiTypeFetchStep> {
    #[instrument(level = "trace", skip_all)]
    pub(crate) fn turn_mutations_into_sequence(&mut self) -> Result<(), FetchGraphError> {
        let root_index = self
            .root_index
            .ok_or(FetchGraphError::NonSingleRootStep(0))?;

        if self.operation_kind != OperationKind::Mutation {
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

        let ordered_roots: Vec<NodeIndex> = node_mutation_field_pos_pairs
            .iter()
            .map(|(idx, _)| *idx)
            .collect();
        let root_set: HashSet<NodeIndex> = ordered_roots.iter().cloned().collect();

        // Capture each root mutation's own result fetches (entity fetches, etc.)
        // BEFORE chaining, so the sets don't include other roots via the chain.
        let mut descendants_per_root: Vec<Vec<NodeIndex>> = Vec::with_capacity(ordered_roots.len());
        for root_mut in &ordered_roots {
            let mut desc = Vec::new();
            let mut bfs = Bfs::new(&self.graph, *root_mut);
            while let Some(nx) = bfs.next(&self.graph) {
                if nx == *root_mut || nx == root_index || root_set.contains(&nx) {
                    continue;
                }
                desc.push(nx);
            }
            descendants_per_root.push(desc);
        }

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

        // Preserve wave barriers for dependency-aware execution:
        // Sequence(M1, Parallel(E1, M2), M3) requires M3 to wait for E1, not just M2.
        // Each Mi waits for descendants of Mj for j <= i-2. The immediate
        // predecessor's descendants stay parallel (M2 || E1, C || F), matching the
        // existing wave overlap; stronger fully-serial ordering is out of scope.
        for (i, mi) in ordered_roots.iter().enumerate() {
            if i < 2 {
                continue;
            }
            for j in 0..=i - 2 {
                for d in descendants_per_root[j].clone() {
                    if d == *mi {
                        continue;
                    }
                    if self.is_descendant_of(d, *mi) {
                        // Mi can already reach D: adding D -> Mi would cycle.
                        continue;
                    }
                    if self.is_descendant_of(*mi, d) {
                        // D can already reach Mi transitively: redundant.
                        continue;
                    }
                    self.connect(d, *mi);
                }
            }
        }

        Ok(())
    }
}
