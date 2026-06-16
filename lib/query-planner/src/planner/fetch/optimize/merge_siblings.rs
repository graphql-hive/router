use std::{
    collections::{HashMap, VecDeque},
    hash::{Hash, Hasher},
};

use petgraph::{graph::NodeIndex, Direction};
use tracing::instrument;

use crate::{
    ast::merge_path::{Condition, MergePath, Segment},
    planner::fetch::{
        error::FetchGraphError,
        fetch_graph::FetchGraph,
        fetch_step_data::{FetchStepData, FetchStepKind},
        optimize::utils::perform_fetch_step_merge,
        state::MultiTypeFetchStep,
    },
    state::supergraph_state::SubgraphName,
};

#[derive(Clone)]
struct SiblingGroupKey {
    service_name: SubgraphName,
    response_path: MergePath,
    condition: Option<Condition>,
    kind: FetchStepKind,
}

impl PartialEq for SiblingGroupKey {
    fn eq(&self, other: &Self) -> bool {
        // Group by response path shape, not field argument hashes. Argument conflicts are
        // checked by can_merge_siblings before any merge is performed.
        self.service_name == other.service_name
            && self.response_path.inner.len() == other.response_path.inner.len()
            && self
                .response_path
                .inner
                .iter()
                .zip(other.response_path.inner.iter())
                .all(|(a, b)| match (a, b) {
                    (Segment::Field(fa, _, ca), Segment::Field(fb, _, cb)) => fa == fb && ca == cb,
                    (Segment::List, Segment::List) => true,
                    (Segment::TypeCondition(ta, ca), Segment::TypeCondition(tb, cb)) => {
                        ta == tb && ca == cb
                    }
                    _ => false,
                })
            && self.condition == other.condition
            && self.kind == other.kind
    }
}

impl Eq for SiblingGroupKey {}

impl Hash for SiblingGroupKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.service_name.hash(state);
        for segment in self.response_path.inner.iter() {
            match segment {
                Segment::Field(fp, _, cond) => {
                    0u8.hash(state);
                    fp.hash(state);
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
        self.condition.hash(state);
        self.kind.hash(state);
    }
}

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
            self.merge_large_sibling_groups(&siblings)?;
        }

        Ok(())
    }

    fn merge_large_sibling_groups(
        &mut self,
        siblings: &[NodeIndex],
    ) -> Result<(), FetchGraphError> {
        let mut groups: HashMap<SiblingGroupKey, Vec<NodeIndex>> = HashMap::new();

        for &sibling_index in siblings {
            if self.graph.node_weight(sibling_index).is_none() {
                continue;
            }

            let step = self.get_step_data(sibling_index)?;

            groups
                .entry(SiblingGroupKey {
                    service_name: step.service_name.clone(),
                    response_path: step.response_path.clone(),
                    condition: if matches!(step.kind, FetchStepKind::Entity) {
                        step.condition.clone()
                    } else {
                        None
                    },
                    kind: step.kind.clone(),
                })
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

        Ok(())
    }
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
