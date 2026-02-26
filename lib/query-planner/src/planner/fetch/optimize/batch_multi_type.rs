use std::collections::{BTreeSet, HashMap, VecDeque};
use std::hash::{DefaultHasher, Hash, Hasher};

use petgraph::{graph::NodeIndex, Direction};
use tracing::{instrument, trace};

use crate::ast::merge_path::Condition;
use crate::planner::fetch::fetch_step_data::FetchStepFlags;
use crate::planner::fetch::optimize::utils::cast_types_from_response_path;
use crate::{
    ast::merge_path::{MergePath, Segment},
    planner::fetch::{
        error::FetchGraphError, fetch_graph::FetchGraph, fetch_step_data::FetchStepData,
        optimize::utils::perform_fetch_step_merge, state::MultiTypeFetchStep,
    },
};

impl FetchGraph<MultiTypeFetchStep> {
    /// Batches sibling entity fetches that only differ by type conditions.
    ///
    /// - find compatible sibling fetches
    /// - merge them into fewer multi-type fetches
    /// - update response_path so Flatten(path) still extracts correct data
    ///
    /// We keep type casts when needed for correctness, and drop them only when
    /// sibling query coverage shows the cast is redundant at that path position.
    ///
    /// Example: at `products.@.reviews.@.product`, if the only sibling fragments
    /// used there are `... on Book` and `... on Magazine`, then merging `|[Book]`
    /// and `|[Magazine]` can drop the cast at that slot.
    #[instrument(level = "trace", skip_all)]
    pub(crate) fn batch_multi_type(&mut self) -> Result<(), FetchGraphError> {
        let root_index = self
            .root_index
            .ok_or(FetchGraphError::NonSingleRootStep(0))?;
        // Breadth-First Search (BFS) starting from the root node.
        let mut queue = VecDeque::from([root_index]);

        while let Some(parent_index) = queue.pop_front() {
            // We only batch child steps that share the same parent.
            let mut merges_to_perform = Vec::<(NodeIndex, NodeIndex)>::new();
            let mut node_indexes: HashMap<NodeIndex, NodeIndex> = HashMap::new();
            let siblings_indices = self
                .graph
                .neighbors_directed(parent_index, Direction::Outgoing)
                .collect::<Vec<NodeIndex>>();
            // For each cast position on sibling paths, collect all requested concrete types.
            // Example: siblings request |[Book] and |[Magazine] at the same position,
            // so coverage for that position is {Book, Magazine}.
            let requested_cast_types_by_position =
                self.requested_cast_types_by_position(&siblings_indices)?;

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

            // First find all merge candidates. Then apply merges.
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

                // Revalidate because previous merges may change step compatibility
                let can_still_batch = {
                    let left = self.get_step_data(*child_index_latest)?;
                    let right = self.get_step_data(*other_child_index_latest)?;
                    left.can_be_batched_with(right)
                };

                if !can_still_batch {
                    continue;
                }

                let (me, other) =
                    self.get_pair_of_steps_mut(*child_index_latest, *other_child_index_latest)?;

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

                perform_fetch_step_merge(
                    *child_index_latest,
                    *other_child_index_latest,
                    self,
                    true,
                )?;

                let merged = self.get_step_data_mut(*child_index_latest)?;
                // After merge, update the path so Flatten(path) is correct.
                merged.response_path = merge_batched_response_paths(
                    &original_me_path,
                    &original_other_path,
                    &requested_cast_types_by_position,
                );
                if merged.is_fetching_multiple_types() {
                    if let Some(condition) = merged.condition.clone() {
                        if let Some(conditioned_types) =
                            cast_types_from_response_path(&merged.response_path)
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

                // Because `other_child` was merged into `child`,
                // then everything that was pointing to `other_child`
                // has to point to the `child`.
                node_indexes.insert(*other_child_index_latest, *child_index_latest);
            }
        }

        Ok(())
    }

    fn requested_cast_types_by_position(
        &self,
        siblings_indices: &[NodeIndex],
    ) -> Result<HashMap<(u64, usize), BTreeSet<String>>, FetchGraphError> {
        // Key: (path hash without casts, cast position).
        // Value: all concrete types requested by siblings at that position.
        let mut result = HashMap::<(u64, usize), BTreeSet<String>>::new();

        for sibling_index in siblings_indices {
            let sibling = self.get_step_data(*sibling_index)?;
            let path = &sibling.response_path;
            let normalized_path_key = normalized_path_hash(path);

            let mut segment_idx = 0usize;
            let mut non_cast_position = 0usize;

            loop {
                let mut cast_members = BTreeSet::<String>::new();
                while let Some(Segment::Cast(type_names, _)) = path.inner.get(segment_idx) {
                    cast_members.extend(type_names.iter().cloned());
                    segment_idx += 1;
                }

                if !cast_members.is_empty() {
                    result
                        .entry((normalized_path_key, non_cast_position))
                        .or_default()
                        .extend(cast_members);
                }

                match path.inner.get(segment_idx) {
                    Some(_) => {
                        segment_idx += 1;
                        non_cast_position += 1;
                    }
                    None => break,
                }
            }
        }

        Ok(result)
    }
}

fn merge_batched_response_paths(
    me: &MergePath,
    other: &MergePath,
    requested_cast_types_by_position: &HashMap<(u64, usize), BTreeSet<String>>,
) -> MergePath {
    // We merge only the cast information and keep the rest of the path identical.
    // If any non-cast structure differs, we bail out and keep `me` unchanged.
    // Consume consecutive Cast segments at the current cursor and return:
    // - whether we consumed at least one cast,
    // - the union of cast member type names.
    // The cursor is advanced to the first non-cast segment.
    fn consume_casts<'a>(path: &'a MergePath, idx: &mut usize) -> (bool, BTreeSet<&'a str>) {
        let mut had_cast = false;
        let mut cast_members = BTreeSet::new();

        while let Some(Segment::Cast(type_names, _)) = path.inner.get(*idx) {
            had_cast = true;
            cast_members.extend(type_names.iter().map(|s| s.as_str()));
            *idx += 1;
        }

        (had_cast, cast_members)
    }

    fn merged_cast_segment(
        normalized_path_key: u64,
        non_cast_position: usize,
        cast_changed: bool,
        me_had_cast: bool,
        other_had_cast: bool,
        merged_cast_members: BTreeSet<&str>,
        requested_cast_types_by_position: &HashMap<(u64, usize), BTreeSet<String>>,
    ) -> Option<Segment> {
        // We keep cast only when both sides were casted at this position.
        if !me_had_cast || !other_had_cast || merged_cast_members.is_empty() {
            return None;
        }

        // If both sides had the same cast already (for example |[Agency]),
        // keep it to preserve type-conditioned extraction semantics.
        // Example: dropping |[Agency] can send Self/Group values to agency fetches.
        if !cast_changed {
            return Some(Segment::Cast(
                merged_cast_members.iter().map(|s| s.to_string()).collect(),
                None,
            ));
        }

        // Strip cast when all queried type branches at this exact path position
        // are already covered by the merged cast members.
        // Example: merged {Book, Magazine} and requested {Book, Magazine} -> strip.
        let requested_types =
            requested_cast_types_by_position.get(&(normalized_path_key, non_cast_position));
        if requested_types.is_some_and(|requested_types| {
            requested_types.len() == merged_cast_members.len()
                && merged_cast_members
                    .iter()
                    .all(|member| requested_types.contains(*member))
        }) {
            return None;
        }

        Some(Segment::Cast(
            merged_cast_members.iter().map(|s| s.to_string()).collect(),
            None,
        ))
    }

    // If non-cast parts differ, do not merge.
    // Example: a.@.b vs a.c.b.
    if me.without_type_castings() != other.without_type_castings() {
        return me.clone();
    }
    let normalized_path_key = normalized_path_hash(me);

    // Final merged path.
    let mut merged = Vec::<Segment>::new();
    // Current position in each path while we walk them together.
    let mut me_idx = 0;
    let mut other_idx = 0;
    let mut non_cast_position = 0usize;

    loop {
        // Read all cast segments at the current position from both paths.
        let (me_had_cast, me_cast_members) = consume_casts(me, &mut me_idx);
        let (other_had_cast, mut other_cast_members) = consume_casts(other, &mut other_idx);
        let cast_changed = me_cast_members != other_cast_members;

        // Combine cast type names from both sides (for example Book + User).
        let mut merged_cast_members = me_cast_members;
        merged_cast_members.append(&mut other_cast_members);

        // Keep or remove cast here based on merged types and sibling usage.
        if let Some(merged_cast_segment) = merged_cast_segment(
            normalized_path_key,
            non_cast_position,
            cast_changed,
            me_had_cast,
            other_had_cast,
            merged_cast_members,
            requested_cast_types_by_position,
        ) {
            merged.push(merged_cast_segment);
        }

        // Non-cast segments must match.
        match (me.inner.get(me_idx), other.inner.get(other_idx)) {
            (Some(me_segment), Some(other_segment)) => {
                if me_segment != other_segment {
                    // Shapes differ here, so merging would be unsafe.
                    return me.clone();
                }

                merged.push(me_segment.clone());
                me_idx += 1;
                other_idx += 1;
                non_cast_position += 1;
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
                    None => "Condition(None)".hash(&mut hasher),
                }
            }
            Segment::List => {
                "List".hash(&mut hasher);
            }
            // Ignore casts when building the normalized path hash.
            Segment::Cast(_, _) => {}
        }
    }

    hasher.finish()
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

        // Paths must match after removing casts.
        if self.response_path.without_type_castings() != other.response_path.without_type_castings()
        {
            return false;
        }

        // Paths must have the same length.
        if self.response_path.len() != other.response_path.len() {
            return false;
        }

        // Cast segments must be in the same positions.
        // Example: a.@|[Book].b and a.@.b are not compatible.
        if self
            .response_path
            .inner
            .iter()
            .zip(other.response_path.inner.iter())
            .any(|(left, right)| {
                matches!(left, Segment::Cast(_, _)) != matches!(right, Segment::Cast(_, _))
            })
        {
            return false;
        }

        // Cast conditions must also match at each position.
        // Example: |[Book] @skip(if: $x) vs |[Book] with no condition.
        if self
            .response_path
            .inner
            .iter()
            .zip(other.response_path.inner.iter())
            .any(|(left, right)| match (left, right) {
                (Segment::Cast(_, left_condition), Segment::Cast(_, right_condition)) => {
                    left_condition != right_condition
                }
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
        ast::merge_path::{MergePath, Segment},
        planner::plan_nodes::FlattenNodePath,
    };

    use super::{merge_batched_response_paths, normalized_path_hash};

    fn path(type_name: &str) -> MergePath {
        MergePath::new(vec![
            Segment::Field("products".to_string(), 0, None),
            Segment::List,
            Segment::Cast(BTreeSet::from([type_name.to_string()]), None),
            Segment::Field("reviews".to_string(), 0, None),
            Segment::List,
            Segment::Field("product".to_string(), 0, None),
        ])
    }

    #[test]
    fn keeps_non_exhaustive_type_list_in_flatten_path() {
        let me = path("Book");
        let other = path("User");
        let normalized_path_key = normalized_path_hash(&me);
        let requested_cast_types_by_position = HashMap::from([(
            (normalized_path_key, 2usize),
            BTreeSet::from_iter([
                "Book".to_string(),
                "User".to_string(),
                "Magazine".to_string(),
            ]),
        )]);

        let merged = merge_batched_response_paths(&me, &other, &requested_cast_types_by_position);

        assert_eq!(
            format!("{}", FlattenNodePath::from(merged)),
            "products.@|[Book|User].reviews.@.product"
        );
    }

    #[test]
    fn strips_type_list_when_casts_are_exhaustive() {
        let me = path("Book");
        let other = path("User");
        let normalized_path_key = normalized_path_hash(&me);
        let requested_cast_types_by_position = HashMap::from([(
            (normalized_path_key, 2usize),
            BTreeSet::from_iter(["Book".to_string(), "User".to_string()]),
        )]);

        let merged = merge_batched_response_paths(&me, &other, &requested_cast_types_by_position);

        assert_eq!(
            format!("{}", FlattenNodePath::from(merged)),
            "products.@.reviews.@.product"
        );
    }
}
