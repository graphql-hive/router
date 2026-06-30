use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
};

use petgraph::graph::NodeIndex;
use rustc_hash::FxHasher;
use tracing::instrument;

use crate::{
    ast::merge_path::{Condition, MergePath, Segment},
    planner::{
        fetch::{
            error::FetchGraphError, fetch_graph::FetchGraph, fetch_step_data::FetchStepData,
            optimize::utils::perform_fetch_step_merge, state::MultiTypeFetchStep,
        },
        tree::query_tree_node::MutationFieldPosition,
    },
    state::supergraph_state::SubgraphName,
};

impl FetchStepData<'_, MultiTypeFetchStep> {
    pub fn can_merge_leafs(
        &self,
        self_index: NodeIndex,
        other_index: NodeIndex,
        other: &Self,
        fetch_graph: &FetchGraph<'_, MultiTypeFetchStep>,
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

#[derive(Clone)]
// Coarse grouping to avoid scanning every node pair. can_merge_leafs still validates graph
// dependencies and argument conflicts before merging.
struct GroupKey<'a> {
    service_name: SubgraphName<'a>,
    response_path: MergePath,
    input_types_hash: u64,
    condition: Option<Condition>,
    mutation_field_position: MutationFieldPosition,
}

struct LeafMergeInfo {
    index: NodeIndex,
    is_leaf: bool,
}

impl PartialEq for GroupKey<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.service_name == other.service_name
            && self.response_path == other.response_path
            && self.input_types_hash == other.input_types_hash
            && self.condition == other.condition
            && self.mutation_field_position == other.mutation_field_position
    }
}

impl Eq for GroupKey<'_> {}

impl Hash for GroupKey<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.service_name.hash(state);
        for segment in self.response_path.inner.iter() {
            match segment {
                Segment::Field(fp, hash, cond) => {
                    0u8.hash(state);
                    fp.hash(state);
                    hash.hash(state);
                    cond.hash(state);
                }
                Segment::List => 1u8.hash(state),
                Segment::TypeCondition(types, cond) => {
                    2u8.hash(state);
                    for t in types {
                        t.hash(state);
                    }
                    cond.hash(state);
                }
            }
        }
        self.input_types_hash.hash(state);
        self.condition.hash(state);
        self.mutation_field_position.hash(state);
    }
}

impl<'a> FetchGraph<'a, MultiTypeFetchStep> {
    #[instrument(level = "trace", skip_all)]
    /// This optimization is about merging leaf nodes in the fetch nodes with other nodes.
    /// It reduces the number of fetch steps, without degrading the query performance.
    /// The query performance is not degraded, because the leaf node has no children,
    /// meaning the overall depth (amount of parallel layers) is not increased.
    pub(crate) fn merge_leafs(&mut self) -> Result<(), FetchGraphError> {
        self.merge_leaf_groups()
    }

    fn merge_leaf_groups(&mut self) -> Result<(), FetchGraphError> {
        let mut groups: HashMap<GroupKey<'a>, Vec<LeafMergeInfo>> = HashMap::new();

        for node_index in self.graph.node_indices() {
            if self.root_index == Some(node_index) {
                continue;
            }

            let step = self.get_step_data(node_index)?;

            let is_leaf = self.children_of(node_index).next().is_none();
            groups
                .entry(GroupKey {
                    service_name: step.service_name.clone(),
                    response_path: step.response_path.clone(),
                    input_types_hash: input_types_hash(step),
                    condition: step.condition.clone(),
                    mutation_field_position: step.mutation_field_position,
                })
                .or_default()
                .push(LeafMergeInfo {
                    index: node_index,
                    is_leaf,
                });
        }

        for group in groups.into_values() {
            // Every item in the group can be a target, but only leaf nodes can be
            // merged into another step. Keep one list instead of separate target
            // and leaf lists, so we allocate less.
            //
            // Keep the target-first order. It makes the output stable because
            // merge order affects selection order in the final printed plan.
            for target in &group {
                if self.graph.node_weight(target.index).is_none() {
                    continue;
                }

                for leaf in group.iter().filter(|info| info.is_leaf) {
                    if target.index == leaf.index
                        || self.graph.node_weight(target.index).is_none()
                        || self.graph.node_weight(leaf.index).is_none()
                    {
                        continue;
                    }

                    let can_merge = {
                        let target_step = self.get_step_data(target.index)?;
                        let leaf_step = self.get_step_data(leaf.index)?;
                        target_step.can_merge_leafs(target.index, leaf.index, leaf_step, self)
                    };

                    if can_merge {
                        perform_fetch_step_merge(target.index, leaf.index, self, false)?;
                    }
                }
            }
        }

        Ok(())
    }
}

fn input_types_hash(step: &FetchStepData<'_, MultiTypeFetchStep>) -> u64 {
    let mut hasher = FxHasher::default();

    for (type_name, _) in step.input.iter_selections() {
        type_name.hash(&mut hasher);
    }

    hasher.finish()
}
