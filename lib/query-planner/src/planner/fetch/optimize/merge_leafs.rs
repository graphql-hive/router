use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
};

use petgraph::graph::NodeIndex;
use rustc_hash::FxHasher;
use tracing::instrument;

use crate::{
    ast::merge_path::{Condition, Segment},
    planner::{
        fetch::{
            error::FetchGraphError, fetch_graph::FetchGraph, fetch_step_data::FetchStepData,
            optimize::utils::perform_fetch_step_merge, state::MultiTypeFetchStep,
        },
        tree::query_tree_node::MutationFieldPosition,
    },
    state::supergraph_state::SubgraphName,
};

impl FetchStepData<MultiTypeFetchStep> {
    pub fn can_merge_leafs(
        &self,
        self_index: NodeIndex,
        other_index: NodeIndex,
        other: &Self,
        fetch_graph: &FetchGraph<MultiTypeFetchStep>,
    ) -> bool {
        if self_index == other_index {
            return false;
        }

        if self.service_name != other.service_name {
            return false;
        }

        if self.response_path != other.response_path {
            return false;
        }

        if !self.input.selecting_same_types(&other.input) {
            return false;
        }

        if self.condition != other.condition {
            return false;
        }

        // otherwise we break the order of mutations
        if self.mutation_field_position != other.mutation_field_position {
            return false;
        }

        // `other` must still be a leaf node (no children).
        if fetch_graph.children_of(other_index).next().is_some() {
            return false;
        }

        // We can't merge if one is a descendant of the other,
        // because there's a dependency between them,
        // that could lead to incorrect results.
        // Either input of "other" depends on the output of "self",
        // or input of "other" depends on the output of one of the steps in between.
        if fetch_graph.is_descendant_of(other_index, self_index) {
            return false;
        }

        if self.has_arguments_conflicts_with(other) {
            return false;
        }

        true
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct GroupKey {
    service_name: SubgraphName,
    response_path_hash: u64,
    input_types_hash: u64,
    condition: Option<Condition>,
    mutation_field_position: MutationFieldPosition,
}

impl FetchGraph<MultiTypeFetchStep> {
    #[instrument(level = "trace", skip_all)]
    /// This optimization is about merging leaf nodes in the fetch nodes with other nodes.
    /// It reduces the number of fetch steps, without degrading the query performance.
    /// The query performance is not degraded, because the leaf node has no children,
    /// meaning the overall depth (amount of parallel layers) is not increased.
    pub(crate) fn merge_leafs(&mut self) -> Result<(), FetchGraphError> {
        let mut groups: HashMap<GroupKey, Vec<(NodeIndex, bool)>> = HashMap::new();

        for node_index in self.graph.node_indices() {
            if self.root_index == Some(node_index) {
                continue;
            }

            let step = self.get_step_data(node_index)?;

            let is_leaf = self.children_of(node_index).next().is_none();
            groups
                .entry(GroupKey {
                    service_name: step.service_name.clone(),
                    response_path_hash: response_path_hash(step),
                    input_types_hash: input_types_hash(step),
                    condition: step.condition.clone(),
                    mutation_field_position: step.mutation_field_position,
                })
                .or_default()
                .push((node_index, is_leaf));
        }

        for group in groups.into_values() {
            // Every item in the group can be a target, but only leaf nodes can be
            // merged into another step. Keep one list instead of separate target
            // and leaf lists, so we allocate less.
            //
            // Keep the target-first order. It makes the output stable because
            // merge order affects selection order in the final printed plan.
            for (target_index, _) in &group {
                if self.graph.node_weight(*target_index).is_none() {
                    continue;
                }

                for (leaf_index, _) in group.iter().filter(|(_, is_leaf)| *is_leaf) {
                    if target_index == leaf_index
                        || self.graph.node_weight(*target_index).is_none()
                        || self.graph.node_weight(*leaf_index).is_none()
                    {
                        continue;
                    }

                    let can_merge = {
                        let target_step = self.get_step_data(*target_index)?;
                        let leaf_step = self.get_step_data(*leaf_index)?;
                        target_step.can_merge_leafs(*target_index, *leaf_index, leaf_step, self)
                    };

                    if can_merge {
                        perform_fetch_step_merge(*target_index, *leaf_index, self, false)?;
                    }
                }
            }
        }

        Ok(())
    }
}

fn input_types_hash(step: &FetchStepData<MultiTypeFetchStep>) -> u64 {
    let mut hasher = FxHasher::default();

    for (type_name, _) in step.input.iter_selections() {
        type_name.hash(&mut hasher);
    }

    hasher.finish()
}

fn response_path_hash(step: &FetchStepData<MultiTypeFetchStep>) -> u64 {
    let mut hasher = FxHasher::default();

    for segment in step.response_path.inner.iter() {
        match segment {
            Segment::Field(fp, hash, cond) => {
                0u8.hash(&mut hasher);
                fp.hash(&mut hasher);
                hash.hash(&mut hasher);
                cond.hash(&mut hasher);
            }
            Segment::List => 1u8.hash(&mut hasher),
            Segment::TypeCondition(types, cond) => {
                2u8.hash(&mut hasher);
                for t in types {
                    t.hash(&mut hasher);
                }
                cond.hash(&mut hasher);
            }
        }
    }

    hasher.finish()
}
