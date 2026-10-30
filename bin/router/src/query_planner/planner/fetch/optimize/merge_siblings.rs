use ahash::{HashMap, HashMapExt};
use std::{
    collections::VecDeque,
    hash::{Hash, Hasher},
};

use petgraph::{graph::NodeIndex, Direction};
use rustc_hash::FxHasher;
use tracing::{instrument, trace};

use crate::query_planner::{
    ast::merge_path::Segment,
    planner::fetch::{
        error::FetchGraphError,
        fetch_graph::FetchGraph,
        fetch_step_data::FetchStepData,
        optimize::utils::{perform_fetch_step_merge, MergedSteps},
        state::MultiTypeFetchStep,
    },
    state::supergraph_state::SubgraphName,
};

type SiblingGroupKey = (SubgraphName, u64);

impl FetchGraph<MultiTypeFetchStep> {
    #[instrument(level = "trace", skip_all)]
    pub(crate) fn merge_siblings(&mut self) -> Result<(), FetchGraphError> {
        let root_index = self
            .root_index
            .ok_or(FetchGraphError::NonSingleRootStep(0))?;
        // Breadth-First Search (BFS) starting from the root node.
        let mut queue = VecDeque::from([root_index]);
        let mut merged_steps = MergedSteps::default();

        while let Some(parent_index) = queue.pop_front() {
            let parent_index = merged_steps.resolve(parent_index);
            // Process siblings in mutation order, preserving the original order for ties.
            let mut siblings_with_position: Vec<(NodeIndex, Option<usize>)> = self
                .graph
                .neighbors_directed(parent_index, Direction::Outgoing)
                .map(|index| {
                    self.get_step_data(index)
                        .map(|step| (index, step.mutation_field_position))
                })
                .collect::<Result<_, _>>()?;
            siblings_with_position.sort_by_key(|(_, position)| *position);
            let siblings: Vec<NodeIndex> = siblings_with_position
                .into_iter()
                .map(|(index, _)| index)
                .collect();

            let mut group_indexes = HashMap::<SiblingGroupKey, usize>::new();
            let mut groups = Vec::<Vec<NodeIndex>>::new();

            for sibling_index in siblings {
                queue.push_back(sibling_index);

                if self.graph.node_weight(sibling_index).is_none() {
                    continue;
                }

                let step = self.get_step_data(sibling_index)?;
                let group_key = (step.service_name.clone(), response_path_shape_hash(step));
                let group_index = match group_indexes.get(&group_key) {
                    Some(&group_index) => group_index,
                    None => {
                        let group_index = groups.len();
                        group_indexes.insert(group_key, group_index);
                        groups.push(Vec::new());
                        group_index
                    }
                };
                groups[group_index].push(sibling_index);
            }

            for mut group in groups {
                // `cage` takes in `cage.|[Cage]` and `pet` takes in `pet @include(if: $a)`, not
                // the other way around
                group.sort_by_cached_key(|index| {
                    self.get_step_data(*index).map_or((0, 0), |step| {
                        (
                            step.response_path.len(),
                            step.response_path.conditions().count(),
                        )
                    })
                });
                let mut targets = Vec::<NodeIndex>::new();

                for source_index in group {
                    if self.graph.node_weight(source_index).is_none() {
                        continue;
                    }

                    let mut merged = false;
                    for target_index in &targets {
                        if self.graph.node_weight(*target_index).is_none() {
                            continue;
                        }

                        trace!(
                            "checking if [{}] and [{}] can be merged",
                            target_index.index(),
                            source_index.index()
                        );

                        let can_merge = {
                            let target = self.get_step_data(*target_index)?;
                            let source = self.get_step_data(source_index)?;
                            target.can_merge_siblings(*target_index, source_index, source, self)
                        };

                        if can_merge {
                            trace!(
                                "Found siblings optimization: {} <- {}",
                                target_index.index(),
                                source_index.index()
                            );
                            perform_fetch_step_merge(*target_index, source_index, self, false)?;
                            merged_steps.record(source_index, *target_index);
                            merged = true;
                            break;
                        }
                    }

                    if !merged {
                        targets.push(source_index);
                    }
                }
            }
        }
        Ok(())
    }
}

fn response_path_shape_hash(step: &FetchStepData<MultiTypeFetchStep>) -> u64 {
    let mut hasher = FxHasher::default();

    // without conditions and the type conditions it ends with, `cage`, `cage.|[Cage]` and
    // `cage @include(if: $a)` may be the same objects, `can_merge` tells
    let segments = &step.response_path.inner;
    let end = segments
        .iter()
        .rposition(|segment| !matches!(segment, Segment::TypeCondition(..)))
        .map_or(0, |last| last + 1);
    for segment in segments[..end].iter() {
        match segment {
            Segment::Field(field_path, _, _) => {
                0u8.hash(&mut hasher);
                field_path.hash(&mut hasher);
            }
            Segment::List => 1u8.hash(&mut hasher),
            Segment::TypeCondition(types, _) => {
                2u8.hash(&mut hasher);
                for type_name in types.iter() {
                    type_name.hash(&mut hasher);
                }
            }
        }
    }

    hasher.finish()
}

impl FetchStepData<MultiTypeFetchStep> {
    pub(crate) fn can_merge_siblings(
        &self,
        self_index: NodeIndex,
        other_index: NodeIndex,
        other: &Self,
        fetch_graph: &FetchGraph<MultiTypeFetchStep>,
    ) -> bool {
        // First, check if the base conditions for merging are met.
        let can_merge_base = self.can_merge(self_index, other_index, other, fetch_graph);

        if fetch_graph.is_ancestor_or_descendant(self_index, other_index) {
            // Looks like they depend on each other
            return false;
        }

        can_merge_base
    }
}
