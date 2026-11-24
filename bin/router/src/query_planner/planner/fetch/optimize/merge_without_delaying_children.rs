use ahash::{HashMap, HashMapExt};
use std::hash::{Hash, Hasher};

use petgraph::{
    algo::toposort,
    graph::NodeIndex,
    visit::{EdgeRef, NodeIndexable},
};
use rustc_hash::FxHasher;
use tracing::instrument;

use crate::query_planner::{
    ast::merge_path::{Condition, Segment},
    planner::fetch::{
        error::FetchGraphError, fetch_graph::FetchGraph, fetch_step_data::FetchStepData,
        optimize::utils::perform_fetch_step_merge, state::MultiTypeFetchStep,
    },
    state::supergraph_state::SubgraphName,
};

impl FetchStepData<MultiTypeFetchStep> {
    pub fn can_merge_at_same_spot(&self, other: &Self) -> bool {
        if !self.can_merge_context(other) {
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

        if !self.inputs_hold_when_merged(other) {
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
}

impl FetchGraph<MultiTypeFetchStep> {
    #[instrument(level = "trace", skip_all)]
    /// Merges steps to the same subgraph at the same spot, when it costs no wave: the merged
    /// step runs when the later of the two did, so every child of both has to run after that.
    /// A leaf only has to fit before the other step's children. Like `c` asked for `x` right
    /// after `b`, and for `y` after `a`, where `d` waits for both: asking `c` for both after `a`
    /// is as fast.
    pub(crate) fn merge_without_delaying_children(&mut self) -> Result<(), FetchGraphError> {
        let mut group_indexes = HashMap::<GroupKey, usize>::new();
        let mut groups = Vec::<Vec<NodeIndex>>::new();

        for node_index in self.graph.node_indices() {
            if self.root_index == Some(node_index) {
                continue;
            }

            let step = self.get_step_data(node_index)?;
            let group_key = GroupKey {
                service_name: step.service_name.clone(),
                response_path_hash: response_path_hash(step),
                input_types_hash: input_types_hash(step),
                condition: step.condition.clone(),
            };
            let group_index = match group_indexes.get(&group_key) {
                Some(&group_index) => group_index,
                None => {
                    let group_index = groups.len();
                    group_indexes.insert(group_key, group_index);
                    groups.push(Vec::new());
                    group_index
                }
            };
            groups[group_index].push(node_index);
        }

        // worked out when first needed, then kept up to date after each merge
        let mut cached_waves: Option<Vec<usize>> = None;
        for group in groups.iter().filter(|group| group.len() > 1) {
            for target_index in group {
                for other_index in group {
                    if target_index == other_index
                        || self.graph.node_weight(*target_index).is_none()
                        || self.graph.node_weight(*other_index).is_none()
                    {
                        continue;
                    }
                    let waves = match &cached_waves {
                        Some(waves) => waves,
                        None => cached_waves.insert(self.waves()?),
                    };
                    // When one step feeds the other, a child on the way runs no later than the
                    // merged step, so this keeps them apart too.
                    let merged_wave = waves[target_index.index()].max(waves[other_index.index()]);
                    let costs_no_wave = self
                        .children_of(*target_index)
                        .chain(self.children_of(*other_index))
                        .all(|edge| waves[edge.target().index()] > merged_wave);
                    if !costs_no_wave {
                        continue;
                    }
                    let target_step = self.get_step_data(*target_index)?;
                    let other_step = self.get_step_data(*other_index)?;
                    if target_step.can_merge_at_same_spot(other_step) {
                        perform_fetch_step_merge(*target_index, *other_index, self, false)?;
                        // The merged step waits for the parents of both, so it runs in the later
                        // of the two waves. Nothing else moves: every child of both ran after
                        // that already, that's what let them merge.
                        if let Some(waves) = &mut cached_waves {
                            waves[target_index.index()] = merged_wave;
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// The round each step runs in: one after the latest of its parents.
    fn waves(&self) -> Result<Vec<usize>, FetchGraphError> {
        let order = toposort(&self.graph, None)
            .map_err(|_| FetchGraphError::Internal("the fetch graph has a cycle".to_string()))?;
        let mut waves = vec![0; self.graph.node_bound()];
        for index in order {
            let wave = self
                .parents_of(index)
                .map(|edge| waves[edge.source().index()] + 1)
                .max()
                .unwrap_or(0);
            waves[index.index()] = wave;
        }
        Ok(waves)
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
            Segment::Field(field_path, arguments_hash, condition) => {
                0u8.hash(&mut hasher);
                field_path.hash(&mut hasher);
                arguments_hash.hash(&mut hasher);
                condition.hash(&mut hasher);
            }
            Segment::List => 1u8.hash(&mut hasher),
            Segment::TypeCondition(types, condition) => {
                2u8.hash(&mut hasher);
                for type_name in types.iter() {
                    type_name.hash(&mut hasher);
                }
                condition.hash(&mut hasher);
            }
        }
    }

    hasher.finish()
}
