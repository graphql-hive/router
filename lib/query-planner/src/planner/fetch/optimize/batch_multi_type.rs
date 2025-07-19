use std::collections::{HashMap, VecDeque};

use petgraph::{graph::NodeIndex, Direction};
use tracing::{instrument, trace};

use crate::planner::fetch::{
    error::FetchGraphError, fetch_graph::FetchGraph, fetch_step_data::FetchStepData,
    optimize::utils::perform_fetch_step_merge, state::MultiTypeFetchStep,
};

impl FetchGraph<MultiTypeFetchStep> {
    #[instrument(level = "trace", skip_all)]
    pub(crate) fn batch_multi_type(&mut self) -> Result<(), FetchGraphError> {
        let root_index = self
            .root_index
            .ok_or(FetchGraphError::NonSingleRootStep(0))?;
        // Breadth-First Search (BFS) starting from the root node.
        let mut queue = VecDeque::from([root_index]);

        while let Some(parent_index) = queue.pop_front() {
            let mut merges_to_perform = Vec::<(NodeIndex, NodeIndex)>::new();
            let mut node_indexes: HashMap<NodeIndex, NodeIndex> = HashMap::new();
            let siblings_indices = self
                .graph
                .neighbors_directed(parent_index, Direction::Outgoing)
                .collect::<Vec<NodeIndex>>();

            for (i, sibling_index) in siblings_indices.iter().enumerate() {
                queue.push_back(*sibling_index);
                let current = self.get_step_data(*sibling_index)?;

                for other_sibling_index in siblings_indices.iter().skip(i + 1) {
                    trace!(
                        "checking if [{}] and [{}] can be batched",
                        sibling_index.index(),
                        other_sibling_index.index()
                    );

                    let other_sibling = self.get_step_data(*other_sibling_index)?;

                    if current.can_be_batched_with(other_sibling) {
                        trace!(
                            "Found multi-type batching optimization: [{}] <- [{}]",
                            sibling_index.index(),
                            other_sibling_index.index()
                        );
                        // Register their original indexes in the map.
                        node_indexes.insert(*sibling_index, *sibling_index);
                        node_indexes.insert(*other_sibling_index, *other_sibling_index);

                        merges_to_perform.push((*sibling_index, *other_sibling_index));
                    }
                }
            }

            for (child_index, other_child_index) in merges_to_perform {
                // Get the latest indexes for the nodes, accounting for previous merges.
                let child_index_latest = node_indexes
                    .get(&child_index)
                    .ok_or(FetchGraphError::IndexMappingLost)?;
                let other_child_index_latest = node_indexes
                    .get(&other_child_index)
                    .ok_or(FetchGraphError::IndexMappingLost)?;

                if child_index_latest == other_child_index_latest {
                    continue;
                }

                if self.is_ancestor_or_descendant(*child_index_latest, *other_child_index_latest) {
                    continue;
                }

                let (me, other) =
                    self.get_pair_of_steps_mut(*child_index_latest, *other_child_index_latest)?;

                // We "declare" the known type for the step, so later merging will be possible into that type instead of failing with an error.
                for (input_type_name, _) in other.input.iter_selections() {
                    me.input.declare_known_type(input_type_name);
                }

                // We "declare" the known type for the step, so later merging will be possible into that type instead of failing with an error.
                for (output_type_name, _) in other.output.iter_selections() {
                    me.output.declare_known_type(output_type_name);
                }

                perform_fetch_step_merge(
                    *child_index_latest,
                    *other_child_index_latest,
                    self,
                    true,
                )?;

                // Because `other_child` was merged into `child`,
                // then everything that was pointing to `other_child`
                // has to point to the `child`.
                node_indexes.insert(*other_child_index_latest, *child_index_latest);
            }
        }

        Ok(())
    }
}

impl FetchStepData<MultiTypeFetchStep> {
    pub fn can_be_batched_with(&self, other: &Self) -> bool {
        if self.kind != other.kind {
            return false;
        }

        if self.service_name != other.service_name {
            return false;
        }

        if !self.is_entity_call() || !other.is_entity_call() {
            return false;
        }

        if self.response_path.without_type_castings() != other.response_path.without_type_castings()
        {
            return false;
        }

        if self.has_arguments_conflicts_with(other) {
            return false;
        }

        true
    }
}
