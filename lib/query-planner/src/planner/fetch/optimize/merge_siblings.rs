use std::{
    collections::{HashMap, VecDeque},
    hash::{Hash, Hasher},
};

use petgraph::{graph::NodeIndex, Direction};
use tracing::instrument;

use crate::{
    ast::merge_path::{Condition, Segment},
    planner::fetch::{
        error::FetchGraphError,
        fetch_graph::FetchGraph,
        fetch_step_data::{FetchStepData, FetchStepKind},
        optimize::utils::perform_fetch_step_merge,
        state::MultiTypeFetchStep,
    },
    state::supergraph_state::SubgraphName,
};

type SiblingGroupKey = (SubgraphName, u64, Option<Condition>, FetchStepKind);

impl FetchGraph<MultiTypeFetchStep> {
    #[instrument(level = "trace", skip_all)]
    pub(crate) fn merge_siblings(&mut self) -> Result<(), FetchGraphError> {
        let root_index = self
            .root_index
            .ok_or(FetchGraphError::NonSingleRootStep(0))?;
        // Breadth-First Search (BFS) starting from the root node.
        let mut queue = VecDeque::from([root_index]);

        while let Some(parent_index) = queue.pop_front() {
            // Sort fetch steps by mutation's field position,
            // to execute mutations in correct order.
            let mut siblings_with_pos: Vec<(NodeIndex, Option<usize>)> = self
                .graph
                .neighbors_directed(parent_index, Direction::Outgoing)
                .map(|sibling| {
                    self.get_step_data(sibling)
                        .map(|data| (sibling, data.mutation_field_position))
                })
                .collect::<Result<_, _>>()?;

            // Sort fetch steps by mutation's field position,
            // to execute mutations in correct order.
            siblings_with_pos.sort_by_key(|(_node, pos)| *pos);

            let siblings: Vec<NodeIndex> =
                siblings_with_pos.into_iter().map(|(idx, _)| idx).collect();

            for sibling_index in &siblings {
                queue.push_back(*sibling_index);
            }

            let mut groups: HashMap<SiblingGroupKey, Vec<NodeIndex>> = HashMap::new();
            for sibling_index in siblings {
                if self.graph.node_weight(sibling_index).is_none() {
                    continue;
                }

                let step = self.get_step_data(sibling_index)?;
                groups
                    .entry((
                        step.service_name.clone(),
                        response_path_shape_hash(step),
                        if matches!(step.kind, FetchStepKind::Entity) {
                            step.condition.clone()
                        } else {
                            None
                        },
                        step.kind.clone(),
                    ))
                    .or_default()
                    .push(sibling_index);
            }

            for group in groups.into_values() {
                let mut targets: Vec<NodeIndex> = Vec::new();

                for source_index in group {
                    if self.graph.node_weight(source_index).is_none() {
                        continue;
                    }

                    let mut merged = false;
                    for &target_index in &targets {
                        if self.graph.node_weight(target_index).is_none() {
                            continue;
                        }

                        let can_merge = {
                            let target = self.get_step_data(target_index)?;
                            let source = self.get_step_data(source_index)?;
                            target.can_merge_siblings(target_index, source_index, source, self)
                        };

                        if can_merge {
                            perform_fetch_step_merge(target_index, source_index, self, false)?;
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
    let mut hasher = rustc_hash::FxHasher::default();

    for segment in step.response_path.inner.iter() {
        match segment {
            Segment::Field(fp, _, cond) => {
                0u8.hash(&mut hasher);
                fp.hash(&mut hasher);
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

        if let (Some(self_mut_idx), Some(other_mut_index)) =
            (self.mutation_field_position, other.mutation_field_position)
        {
            // If indexes are equal or one happens to be after the other,
            // and we already know they belong to the same service,
            // we shouldn't prevent merging.
            if self_mut_idx != other_mut_index
                && (self_mut_idx as i64 - other_mut_index as i64).abs() != 1
            {
                return false;
            }
        }

        if fetch_graph.is_ancestor_or_descendant(self_index, other_index) {
            // Looks like they depend on each other
            return false;
        }

        can_merge_base
    }
}
