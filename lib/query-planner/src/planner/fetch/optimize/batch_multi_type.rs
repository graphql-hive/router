use std::collections::{BTreeSet, HashMap, VecDeque};
use std::hash::{DefaultHasher, Hash, Hasher};

use petgraph::{graph::NodeIndex, Direction};
use tracing::{instrument, trace};

use crate::ast::merge_path::Condition;
use crate::planner::fetch::fetch_step_data::{
    type_condition_types_from_response_path, FetchStepFlags, FetchStepKind,
};
use crate::{
    ast::merge_path::{MergePath, Segment},
    planner::fetch::{
        error::FetchGraphError, fetch_graph::FetchGraph, fetch_step_data::FetchStepData,
        optimize::utils::perform_fetch_step_merge, state::MultiTypeFetchStep,
    },
    state::supergraph_state::SubgraphName,
};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct BatchKey {
    kind: FetchStepKind,
    service_name: SubgraphName,
    normalized_path_hash: u64,
    response_path_len: usize,
    type_condition_layout_hash: u64,
    used_for_requires: bool,
}

impl BatchKey {
    // Coarse grouping only; can_be_batched_with remains the correctness check.
    fn from_step(
        step: &FetchStepData<MultiTypeFetchStep>,
        normalized_path_hash: u64,
        type_condition_layout_hash: u64,
    ) -> Option<Self> {
        if !step.is_entity_call() {
            return None;
        }

        Some(Self {
            kind: step.kind.clone(),
            service_name: step.service_name.clone(),
            normalized_path_hash,
            response_path_len: step.response_path.len(),
            type_condition_layout_hash,
            used_for_requires: step.flags.contains(FetchStepFlags::USED_FOR_REQUIRES),
        })
    }
}

#[derive(Debug)]
struct SiblingBatchInfo {
    index: NodeIndex,
    // This is used in a few places in this pass. Compute it once per sibling.
    normalized_path_hash: u64,
    // None means this sibling cannot be part of multi-type batching.
    batch_key: Option<BatchKey>,
}

impl FetchGraph<MultiTypeFetchStep> {
    /// Batches sibling entity fetches that only differ by type conditions.
    ///
    /// 1. Find compatible sibling fetches
    /// 2. Merge them into fewer multi-type fetches
    /// 3. Update response_path so Flatten(path) still extracts correct data
    ///
    /// We keep type conditions when needed for correctness, and drop them only when
    /// sibling query coverage shows the type condition is redundant at that path position.
    ///
    /// Example: at `products.@.reviews.@.product`, if the only sibling fragments
    /// used there are `... on Book` and `... on Magazine`, then merging `|[Book]`
    /// and `|[Magazine]` can drop the type condition at that slot.
    #[instrument(level = "trace", skip_all)]
    pub(crate) fn batch_multi_type(&mut self) -> Result<(), FetchGraphError> {
        let root_index = self
            .root_index
            .ok_or(FetchGraphError::NonSingleRootStep(0))?;
        // Breadth-First Search (BFS) starting from the root node.
        let mut queue = VecDeque::from([root_index]);

        while let Some(parent_index) = queue.pop_front() {
            // We only batch child steps that share the same parent.
            let siblings_indices = self
                .graph
                .neighbors_directed(parent_index, Direction::Outgoing)
                .collect::<Vec<NodeIndex>>();

            for sibling_index in siblings_indices.iter() {
                queue.push_back(*sibling_index);
            }

            let sibling_infos = self.sibling_batch_infos(&siblings_indices)?;

            let mut sibling_groups: HashMap<BatchKey, Vec<usize>> = HashMap::new();
            for (info_index, info) in sibling_infos.iter().enumerate() {
                if let Some(batch_key) = info.batch_key.clone() {
                    sibling_groups
                        .entry(batch_key)
                        .or_default()
                        .push(info_index);
                }
            }

            // If every group has one item, no merge is possible.
            // In that case, skip the coverage map because it is only used when merging.
            if !sibling_groups.values().any(|group| group.len() > 1) {
                continue;
            }

            // For each type-condition position on sibling paths, collect all requested concrete types.
            // Example: siblings request |[Book] and |[Magazine] at the same position,
            // so coverage for that position is {Book, Magazine}.
            let requested_type_condition_types_by_position =
                self.requested_type_condition_types_by_position(&sibling_infos)?;

            for sibling_group in sibling_groups.values() {
                if sibling_group.len() < 2 {
                    continue;
                }

                self.merge_multi_type_sibling_group(
                    &sibling_infos,
                    sibling_group,
                    &requested_type_condition_types_by_position,
                )?;
            }
        }

        Ok(())
    }

    fn sibling_batch_infos(
        &self,
        siblings_indices: &[NodeIndex],
    ) -> Result<Vec<SiblingBatchInfo>, FetchGraphError> {
        siblings_indices
            .iter()
            .map(|sibling_index| {
                let current = self.get_step_data(*sibling_index)?;
                let normalized_path_hash = normalized_path_hash(&current.response_path);
                let batch_key = if current.is_entity_call() {
                    BatchKey::from_step(
                        current,
                        normalized_path_hash,
                        type_condition_layout_hash(&current.response_path),
                    )
                } else {
                    None
                };

                Ok(SiblingBatchInfo {
                    index: *sibling_index,
                    normalized_path_hash,
                    batch_key,
                })
            })
            .collect()
    }

    fn merge_multi_type_sibling_group(
        &mut self,
        sibling_infos: &[SiblingBatchInfo],
        sibling_group: &[usize],
        requested_type_condition_types_by_position: &HashMap<(u64, usize), BTreeSet<String>>,
    ) -> Result<(), FetchGraphError> {
        let mut targets: Vec<usize> = Vec::new();

        // We merge sources into live targets as we go.
        // This avoids building a large list of all possible pairs first.
        // It also avoids a map from old node indexes to latest node indexes.
        // The target node stays alive after a merge. The source node is removed.
        for source_info_index in sibling_group {
            let source_info = &sibling_infos[*source_info_index];
            if self.graph.node_weight(source_info.index).is_none() {
                continue;
            }

            let mut merged = false;

            for target_info_index in &targets {
                let target_info = &sibling_infos[*target_info_index];
                if self.graph.node_weight(target_info.index).is_none() {
                    continue;
                }

                trace!(
                    "checking if [{}] and [{}] can be batched",
                    target_info.index.index(),
                    source_info.index.index()
                );

                if self.is_ancestor_or_descendant(target_info.index, source_info.index) {
                    continue;
                }

                // Revalidate here because previous merges can change a target step.
                // A pair that looked compatible earlier may not be compatible anymore.
                let can_batch = {
                    let target = self.get_step_data(target_info.index)?;
                    let source = self.get_step_data(source_info.index)?;
                    target.can_be_batched_with(source)
                };

                if !can_batch {
                    continue;
                }

                trace!(
                    "Found multi-type batching optimization: [{}] <- [{}]",
                    target_info.index.index(),
                    source_info.index.index()
                );

                self.merge_multi_type_sibling_steps(
                    target_info.index,
                    source_info.index,
                    target_info.normalized_path_hash,
                    requested_type_condition_types_by_position,
                )?;
                merged = true;
                break;
            }

            // This source could not merge into any existing target.
            // Keep it as a new target for later sources in the same group.
            if !merged {
                targets.push(*source_info_index);
            }
        }

        Ok(())
    }

    fn merge_multi_type_sibling_steps(
        &mut self,
        target_index: NodeIndex,
        source_index: NodeIndex,
        normalized_path_hash: u64,
        requested_type_condition_types_by_position: &HashMap<(u64, usize), BTreeSet<String>>,
    ) -> Result<(), FetchGraphError> {
        let (me, other) = self.get_pair_of_steps_mut(target_index, source_index)?;

        let original_me_path = me.response_path.clone();
        let original_other_path = other.response_path.clone();

        // We "declare" the known type for the step, so later merging will be possible into that type instead of failing with an error.
        for (input_type_name, _) in other.input.iter_selections() {
            me.input.declare_known_type(input_type_name);
        }

        // We "declare" the known type for the step, so later merging will be possible into that type instead of failing with an error.
        for (output_type_name, _) in other.output.iter_selections() {
            me.output.declare_known_type(output_type_name);
        }

        // This rewires parents and children from source to target, then removes source.
        // After this point, target_index is the live node for the merged fetch.
        perform_fetch_step_merge(target_index, source_index, self, true)?;

        let merged = self.get_step_data_mut(target_index)?;
        // After merge, update the path so Flatten(path) is correct.
        // Example:
        //   `me`     path: products.[Book].reviews
        //   `other`  path: products.[User].reviews
        //   `merged` path: products.[Book|User].reviews (or stripped if fully covered)
        //
        // Without recomputing this path, merged results can be flattened
        // at the wrong location.
        merged.response_path = merge_batched_response_paths(
            &original_me_path,
            &original_other_path,
            normalized_path_hash,
            requested_type_condition_types_by_position,
        );
        if merged.is_fetching_multiple_types() {
            if let Some(condition) = merged.condition.clone() {
                if let Some(conditioned_types) =
                    type_condition_types_from_response_path(&merged.response_path)
                {
                    // Multi-type merged step: keep condition on matching
                    // type branches instead of gating the whole fetch step.
                    merged
                        .output
                        .wrap_with_condition_for_types(condition, &conditioned_types);
                    merged.condition = None;
                }
            }
        }

        Ok(())
    }

    fn requested_type_condition_types_by_position(
        &self,
        sibling_infos: &[SiblingBatchInfo],
    ) -> Result<HashMap<(u64, usize), BTreeSet<String>>, FetchGraphError> {
        // Key: (path hash without type conditions, type-condition position).
        // Value: all concrete types requested by siblings at that position.
        let mut result = HashMap::<(u64, usize), BTreeSet<String>>::new();

        // Iterate over all siblings (fetch steps)
        for sibling_info in sibling_infos {
            let sibling = self.get_step_data(sibling_info.index)?;
            let path = &sibling.response_path;

            // Index of the current non-type-condition segment.
            let mut non_type_condition_position = 0;
            // Collect type names until we hit a non-type-condition segment.
            let mut pending_type_condition_members = BTreeSet::<String>::new();

            // Now let's iterate over segments
            for segment in path.inner.iter() {
                match segment {
                    Segment::TypeCondition(type_names, _) => {
                        // Still at the same slot, so let's add type names from this condition
                        pending_type_condition_members.extend(type_names.iter().cloned());
                    }
                    _ => {
                        if !pending_type_condition_members.is_empty() {
                            // We reached a non-type segment, so save collected types for this slot
                            result
                                .entry((
                                    sibling_info.normalized_path_hash,
                                    non_type_condition_position,
                                ))
                                .or_default()
                                .extend(pending_type_condition_members.iter().cloned());
                            pending_type_condition_members.clear();
                        }

                        // Move to the next non-type slot
                        non_type_condition_position += 1;
                    }
                }
            }

            // If path ends with type conditions, save them for the last slot
            if !pending_type_condition_members.is_empty() {
                result
                    .entry((
                        sibling_info.normalized_path_hash,
                        non_type_condition_position,
                    ))
                    .or_default()
                    .extend(pending_type_condition_members);
            }
        }

        Ok(result)
    }
}

fn merge_batched_response_paths(
    me: &MergePath,
    other: &MergePath,
    normalized_path_key: u64,
    requested_type_condition_types_by_position: &HashMap<(u64, usize), BTreeSet<String>>,
) -> MergePath {
    // Merge rule at each slot (type-condition part only):
    // - If non-type-condition shape differs -> keep `me` unchanged
    // - If both sides already have the same type condition -> keep it
    // - If merged type set fully covers sibling-requested types there -> strip it
    // - Otherwise -> keep merged type set
    //
    // Strip example:
    //   merged types: {Book, Magazine}
    //   requested sibling types: {Book, Magazine}
    //   result: no type-condition segment at this slot.
    //
    // Keep example:
    //   merged types: {Book, User}
    //   requested sibling types: {Book, User, Magazine}
    //   result: keep |[Book|User] at this slot.
    //
    // We merge only type-condition information and keep the rest of the path identical.
    fn consume_type_conditions<'a>(
        path: &'a MergePath,
        idx: &mut usize,
    ) -> (bool, BTreeSet<&'a str>) {
        let mut had_type_condition = false;
        let mut type_condition_members = BTreeSet::new();

        while let Some(Segment::TypeCondition(type_names, _)) = path.inner.get(*idx) {
            had_type_condition = true;
            type_condition_members.extend(type_names.iter().map(|s| s.as_str()));
            *idx += 1;
        }

        (had_type_condition, type_condition_members)
    }

    fn merged_type_condition_segment(
        normalized_path_key: u64,
        non_type_condition_position: usize,
        type_condition_changed: bool,
        me_had_type_condition: bool,
        other_had_type_condition: bool,
        merged_type_condition_members: BTreeSet<&str>,
        requested_type_condition_types_by_position: &HashMap<(u64, usize), BTreeSet<String>>,
    ) -> Option<Segment> {
        // We keep a type condition only when both sides had one at this position.
        if !me_had_type_condition
            || !other_had_type_condition
            || merged_type_condition_members.is_empty()
        {
            return None;
        }

        // If both sides had the same type condition already (for example |[Agency]),
        // keep it to preserve type-conditioned extraction semantics.
        // Example: dropping |[Agency] can send Self/Group values to agency fetches.
        if !type_condition_changed {
            return Some(Segment::TypeCondition(
                merged_type_condition_members
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
                None,
            ));
        }

        // Strip type condition when all queried type branches at this exact path
        // position are already covered by merged type-condition members.
        // Example: merged {Book, Magazine} and requested {Book, Magazine} -> strip.
        let requested_types = requested_type_condition_types_by_position
            .get(&(normalized_path_key, non_type_condition_position));
        if requested_types.is_some_and(|requested_types| {
            requested_types.len() == merged_type_condition_members.len()
                && merged_type_condition_members
                    .iter()
                    .all(|member| requested_types.contains(*member))
        }) {
            return None;
        }

        Some(Segment::TypeCondition(
            merged_type_condition_members
                .iter()
                .map(|s| s.to_string())
                .collect(),
            None,
        ))
    }

    // If non-type-condition parts differ, do not merge.
    // Example: a.@.b vs a.c.b.
    if !same_path_without_type_conditions(me, other) {
        return me.clone();
    }

    // Final merged path.
    let mut merged = Vec::<Segment>::new();
    // Current position in each path while we walk them together.
    let mut me_idx = 0;
    let mut other_idx = 0;
    let mut non_type_condition_position = 0;

    loop {
        // Read all type-condition segments at the current position from both paths.
        let (me_had_type_condition, me_type_condition_members) =
            consume_type_conditions(me, &mut me_idx);
        let (other_had_type_condition, mut other_type_condition_members) =
            consume_type_conditions(other, &mut other_idx);
        let type_condition_changed = me_type_condition_members != other_type_condition_members;

        // Combine type-condition type names from both sides (for example Book + User).
        let mut merged_type_condition_members = me_type_condition_members;
        merged_type_condition_members.append(&mut other_type_condition_members);

        // Keep or remove type condition here based on merged types and sibling usage.
        if let Some(merged_type_condition_segment) = merged_type_condition_segment(
            normalized_path_key,
            non_type_condition_position,
            type_condition_changed,
            me_had_type_condition,
            other_had_type_condition,
            merged_type_condition_members,
            requested_type_condition_types_by_position,
        ) {
            merged.push(merged_type_condition_segment);
        }

        // Non-type-condition segments must match.
        match (me.inner.get(me_idx), other.inner.get(other_idx)) {
            (Some(me_segment), Some(other_segment)) => {
                if me_segment != other_segment {
                    // Shapes differ here, so merging would be unsafe.
                    return me.clone();
                }

                merged.push(me_segment.clone());
                me_idx += 1;
                other_idx += 1;
                non_type_condition_position += 1;
            }
            // Both paths ended together.
            (None, None) => break,
            // One path ended earlier than the other.
            _ => return me.clone(),
        }
    }

    MergePath::new(merged)
}

fn normalized_path_hash(path: &MergePath) -> u64 {
    let mut hasher = DefaultHasher::new();

    for segment in path.inner.iter() {
        match segment {
            Segment::Field(field_name, args_hash, condition) => {
                "Field".hash(&mut hasher);
                field_name.hash(&mut hasher);
                args_hash.hash(&mut hasher);
                match condition {
                    Some(Condition::Skip(variable)) => {
                        "Condition(Skip)".hash(&mut hasher);
                        variable.hash(&mut hasher);
                    }
                    Some(Condition::Include(variable)) => {
                        "Condition(Include)".hash(&mut hasher);
                        variable.hash(&mut hasher);
                    }
                    Some(Condition::SkipAndInclude { skip, include }) => {
                        "Condition(SkipAndInclude)".hash(&mut hasher);
                        skip.hash(&mut hasher);
                        include.hash(&mut hasher);
                    }
                    None => "Condition(None)".hash(&mut hasher),
                }
            }
            Segment::List => {
                "List".hash(&mut hasher);
            }
            // Ignore type conditions when building the normalized path hash.
            Segment::TypeCondition(_, _) => {}
        }
    }

    hasher.finish()
}

fn type_condition_layout_hash(path: &MergePath) -> u64 {
    let mut hasher = DefaultHasher::new();

    for (position, segment) in path.inner.iter().enumerate() {
        if let Segment::TypeCondition(_, condition) = segment {
            position.hash(&mut hasher);
            condition.hash(&mut hasher);
        }
    }

    hasher.finish()
}

fn same_path_without_type_conditions(left: &MergePath, right: &MergePath) -> bool {
    left.inner
        .iter()
        .filter(|segment| !matches!(segment, Segment::TypeCondition(_, _)))
        .eq(right
            .inner
            .iter()
            .filter(|segment| !matches!(segment, Segment::TypeCondition(_, _))))
}

impl FetchStepData<MultiTypeFetchStep> {
    pub fn can_be_batched_with(&self, other: &Self) -> bool {
        // Both steps must be the same fetch kind.
        if self.kind != other.kind {
            return false;
        }

        // Both steps must call the same service.
        if self.service_name != other.service_name {
            return false;
        }

        // Only entity fetches can be batched.
        if !self.is_entity_call() || !other.is_entity_call() {
            return false;
        }

        // Paths must match after removing type conditions.
        if !same_path_without_type_conditions(&self.response_path, &other.response_path) {
            return false;
        }

        // Paths must have the same length.
        // Example: a.@.b and a.@.b.c are not compatible.
        if self.response_path.len() != other.response_path.len() {
            return false;
        }

        // Type-condition segments must be in the same positions.
        // Example: a.@|[Book].b and a.@.b are not compatible.
        if self
            .response_path
            .inner
            .iter()
            .zip(other.response_path.inner.iter())
            .any(|(left, right)| {
                matches!(left, Segment::TypeCondition(_, _))
                    != matches!(right, Segment::TypeCondition(_, _))
            })
        {
            return false;
        }

        // Type-condition conditions must also match at each position.
        // Example: |[Book] @skip(if: $x) vs |[Book] with no condition.
        if self
            .response_path
            .inner
            .iter()
            .zip(other.response_path.inner.iter())
            .any(|(left, right)| match (left, right) {
                (
                    Segment::TypeCondition(_, left_condition),
                    Segment::TypeCondition(_, right_condition),
                ) => left_condition != right_condition,
                _ => false,
            })
        {
            return false;
        }

        if self.has_arguments_conflicts_with(other) {
            return false;
        }

        let self_used_for_requires = self.flags.contains(FetchStepFlags::USED_FOR_REQUIRES);
        let other_used_for_requires = other.flags.contains(FetchStepFlags::USED_FOR_REQUIRES);
        // Mixing @requires and non-@requires steps can widen paths incorrectly.
        // Keep them separate to avoid regressions.
        if self_used_for_requires != other_used_for_requires {
            return false;
        }

        true
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, HashMap};

    use crate::{
        ast::merge_path::{FieldPathSegment, MergePath, Segment},
        planner::plan_nodes::FlattenNodePath,
    };

    use super::{merge_batched_response_paths, normalized_path_hash};

    fn path(type_name: &str) -> MergePath {
        MergePath::new(vec![
            Segment::Field(FieldPathSegment::named("products".to_string()), 0, None),
            Segment::List,
            Segment::TypeCondition(BTreeSet::from([type_name.to_string()]), None),
            Segment::Field(FieldPathSegment::named("reviews".to_string()), 0, None),
            Segment::List,
            Segment::Field(FieldPathSegment::named("product".to_string()), 0, None),
        ])
    }

    #[test]
    fn keeps_non_exhaustive_type_list_in_flatten_path() {
        let me = path("Book");
        let other = path("User");
        let normalized_path_key = normalized_path_hash(&me);
        let requested_type_condition_types_by_position = HashMap::from([(
            (normalized_path_key, 2),
            BTreeSet::from_iter([
                "Book".to_string(),
                "User".to_string(),
                "Magazine".to_string(),
            ]),
        )]);

        let merged = merge_batched_response_paths(
            &me,
            &other,
            normalized_path_key,
            &requested_type_condition_types_by_position,
        );

        assert_eq!(
            format!("{}", FlattenNodePath::from(merged)),
            "products.@|[Book|User].reviews.@.product"
        );
    }

    #[test]
    fn strips_type_list_when_type_conditions_are_exhaustive() {
        let me = path("Book");
        let other = path("User");
        let normalized_path_key = normalized_path_hash(&me);
        let requested_type_condition_types_by_position = HashMap::from([(
            (normalized_path_key, 2),
            BTreeSet::from_iter(["Book".to_string(), "User".to_string()]),
        )]);

        let merged = merge_batched_response_paths(
            &me,
            &other,
            normalized_path_key,
            &requested_type_condition_types_by_position,
        );

        assert_eq!(
            format!("{}", FlattenNodePath::from(merged)),
            "products.@.reviews.@.product"
        );
    }
}
