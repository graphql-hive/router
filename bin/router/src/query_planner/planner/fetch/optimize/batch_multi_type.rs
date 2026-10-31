use ahash::{HashMap, HashMapExt, HashSet};
use std::collections::{BTreeSet, VecDeque};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::rc::Rc;

use petgraph::{graph::NodeIndex, Direction};
use tracing::{instrument, trace};

use crate::query_planner::ast::merge_path::Condition;
use crate::query_planner::ast::selection_set::response_keys_conflict;
use crate::query_planner::planner::fetch::fetch_step_data::{
    type_condition_types_from_response_path, FetchStepFlags, FetchStepKind,
};
use crate::query_planner::planner::fetch::selections::FetchStepSelections;
use crate::query_planner::state::supergraph_state::{SubgraphName, SupergraphState, TypeNode};
use crate::query_planner::{
    ast::merge_path::{MergePath, Segment},
    planner::fetch::{
        error::FetchGraphError,
        fetch_graph::FetchGraph,
        fetch_step_data::FetchStepData,
        optimize::utils::{perform_fetch_step_merge, MergedSteps},
        state::MultiTypeFetchStep,
    },
};

#[derive(Clone, PartialEq, Eq, Hash)]
struct BatchKey {
    kind: FetchStepKind,
    service_name: SubgraphName,
    normalized_path_hash: u64,
    response_path_len: usize,
    type_condition_layout_hash: u64,
    used_for_requires: bool,
}

impl BatchKey {
    // This is only a coarse grouping key; `can_be_batched_with` remains the correctness check.
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

impl FetchGraph<MultiTypeFetchStep> {
    /// Batches sibling entity fetches that only differ by type conditions.
    ///
    /// 1. Find compatible sibling fetches
    /// 2. Merge them into fewer multi-type fetches
    /// 3. Update response_path so Flatten(path) still extracts correct data
    ///
    /// We keep type conditions, and drop one only when the merged types are every type the
    /// value at that path position can be, and it has no `@include`/`@skip`.
    ///
    /// Example: at `products.@.reviews.@.product`, if `Product` is `Book | Magazine`, then
    /// merging `|[Book]` and `|[Magazine]` can drop the type condition at that slot.
    #[instrument(level = "trace", skip_all)]
    pub(crate) fn batch_multi_type(
        &mut self,
        supergraph: &SupergraphState,
    ) -> Result<(), FetchGraphError> {
        let root_type = supergraph
            .root_type_name(Some(&self.operation_kind))
            .map_err(|err| FetchGraphError::Internal(err.to_string()))?;
        let root_index = self
            .root_index
            .ok_or(FetchGraphError::NonSingleRootStep(0))?;
        // Breadth-First Search (BFS) starting from the root node.
        let mut queue = VecDeque::from([root_index]);
        let mut merged_steps = MergedSteps::default();

        while let Some(parent_index) = queue.pop_front() {
            let parent_index = merged_steps.resolve(parent_index);
            // We only batch child steps that share the same parent.
            let siblings_indices = self
                .graph
                .neighbors_directed(parent_index, Direction::Outgoing)
                .collect::<Vec<NodeIndex>>();

            // The map is for lookup; keeping groups in a Vec preserves sibling order and
            // therefore deterministic merge/selection order across planner instances.
            let mut group_indexes = HashMap::<BatchKey, usize>::new();
            let mut groups = Vec::<Vec<(NodeIndex, u64)>>::new();
            for sibling_index in &siblings_indices {
                queue.push_back(*sibling_index);

                if self.graph.node_weight(*sibling_index).is_none() {
                    continue;
                }

                let current = self.get_step_data(*sibling_index)?;
                let normalized_path = normalized_path_hash(&current.response_path);
                let group_key = BatchKey::from_step(
                    current,
                    normalized_path,
                    type_condition_layout_hash(&current.response_path),
                );

                let Some(group_key) = group_key else {
                    continue;
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
                groups[group_index].push((*sibling_index, normalized_path));
            }

            // If each candidate group contains only one step, no pair can be batched.
            if !groups.iter().any(|group| group.len() > 1) {
                continue;
            }

            for group in groups {
                let mut targets = Vec::<(NodeIndex, u64)>::new();

                for (source_index, source_normalized_path) in group {
                    if self.graph.node_weight(source_index).is_none() {
                        continue;
                    }

                    let mut merged = false;
                    for (target_index, target_normalized_path) in &targets {
                        if self.graph.node_weight(*target_index).is_none() {
                            continue;
                        }

                        trace!(
                            "checking if [{}] and [{}] can be batched",
                            target_index.index(),
                            source_index.index()
                        );

                        // Revalidate because previous merges may change step compatibility.
                        let can_still_batch = {
                            let left = self.get_step_data(*target_index)?;
                            let right = self.get_step_data(source_index)?;
                            left.can_be_batched_with(right, supergraph)
                        };

                        // After the cheaper check, as it walks the graph.
                        if !can_still_batch
                            || self.is_ancestor_or_descendant(*target_index, source_index)
                        {
                            continue;
                        }

                        trace!(
                            "Found multi-type batching optimization: [{}] <- [{}]",
                            target_index.index(),
                            source_index.index()
                        );

                        let (me, other) =
                            self.get_pair_of_steps_mut(*target_index, source_index)?;
                        let original_me_path = me.response_path.clone();
                        let original_other_path = other.response_path.clone();
                        let possible_types_by_position = possible_types_by_position(
                            supergraph,
                            root_type,
                            &original_me_path,
                            *target_normalized_path,
                        );

                        // Declare the source types on the target so subsequent compatible
                        // merges can add selections for those types.
                        for (input_type_name, _) in other.input.iter_selections() {
                            me.input.declare_known_type(input_type_name);
                        }
                        for (output_type_name, _) in other.output.iter_selections() {
                            me.output.declare_known_type(output_type_name);
                        }

                        perform_fetch_step_merge(*target_index, source_index, self, true)?;

                        let merged_step = self.get_step_data_mut(*target_index)?;
                        // After merge, update the path so Flatten(path) is correct.
                        // Example:
                        //   `me`     path: products.[Book].reviews
                        //   `other`  path: products.[User].reviews
                        //   `merged` path: products.[Book|User].reviews (or stripped if fully covered)
                        //
                        // Without recomputing this path, merged results can be flattened
                        // at the wrong location.
                        merged_step.response_path = merge_batched_response_paths(
                            &original_me_path,
                            &original_other_path,
                            *target_normalized_path,
                            &possible_types_by_position,
                        );
                        if merged_step.is_fetching_multiple_types() {
                            if let Some(condition) = merged_step.condition.clone() {
                                if let Some(conditioned_types) =
                                    type_condition_types_from_response_path(
                                        &merged_step.response_path,
                                    )
                                {
                                    // Keep a fetch-level condition on only the matching type
                                    // branches rather than gating the entire batched fetch.
                                    merged_step.output.wrap_with_condition_for_types(
                                        condition,
                                        &conditioned_types,
                                    );
                                    merged_step.condition = None;
                                }
                            }
                        }

                        merged_steps.record(source_index, *target_index);
                        merged = true;
                        break;
                    }

                    if !merged {
                        targets.push((source_index, source_normalized_path));
                    }
                }
            }
        }

        Ok(())
    }
}

/// Every object type the value at each slot of `path` can be, keyed like the type conditions
/// `merge_batched_response_paths` looks at: (path hash without type conditions, slot).
/// A slot is left out when its type can't be worked out, and then nothing gets stripped there.
/// The sets are borrowed from the supergraph, cloning them per merge was most of the time.
fn possible_types_by_position<'a>(
    supergraph: &'a SupergraphState,
    root_type: &str,
    path: &MergePath,
    normalized_path_key: u64,
) -> HashMap<(u64, usize), &'a HashSet<String>> {
    let mut result = HashMap::new();
    // The named type of the value at the current slot, and the type condition on it.
    let mut current_type = Some(root_type.to_string());
    let mut type_condition: Option<&BTreeSet<String>> = None;
    let mut position = 0;

    for segment in path.inner.iter() {
        match segment {
            Segment::TypeCondition(type_names, _) => type_condition = Some(type_names),
            Segment::List => position += 1,
            Segment::Field(field, _, _) => {
                let parent_type = type_condition
                    .and_then(|type_names| type_names.first().cloned())
                    .or(current_type);
                current_type = parent_type.and_then(|parent_type| {
                    supergraph
                        .field_return_type_name(&parent_type, field.field_name())
                        .map(str::to_string)
                });
                type_condition = None;
                position += 1;
            }
        }

        if matches!(segment, Segment::TypeCondition(..)) {
            continue;
        }
        if let Some(possible_types) = current_type
            .as_deref()
            .and_then(|type_name| supergraph.all_possible_types(type_name))
            // An interface nothing implements has an empty set. "Every type is merged" would be
            // true for it, so leave it out and keep the type condition.
            .filter(|possible_types| !possible_types.is_empty())
        {
            result.insert((normalized_path_key, position), possible_types);
        }
    }

    result
}

fn merge_batched_response_paths(
    me: &MergePath,
    other: &MergePath,
    normalized_path_key: u64,
    possible_types_by_position: &HashMap<(u64, usize), &HashSet<String>>,
) -> MergePath {
    // Merge rule at each slot (type-condition part only):
    // - If non-type-condition shape differs -> keep `me` unchanged
    // - If both sides already have the same type condition -> keep it
    // - If merged type set is every possible type there -> strip it
    // - Otherwise -> keep merged type set
    //
    // Strip example:
    //   merged types: {Book, Magazine}
    //   possible types: {Book, Magazine}
    //   result: no type-condition segment at this slot.
    //
    // Keep example:
    //   merged types: {Book, User}
    //   possible types: {Book, User, Magazine}
    //   result: keep |[Book|User] at this slot.
    //
    // A type condition keeps its `@include`/`@skip`, and one that has it is never stripped.
    // The fetch runs only when the condition holds, like `... on Product @include(if: $a)`.
    // Its parent only fetches the keys when `$a` is true, so without `$a`, the fetch would
    // send representations without `__typename`.
    //
    // We merge only type-condition information and keep the rest of the path identical.
    fn consume_type_conditions<'a>(
        path: &'a MergePath,
        idx: &mut usize,
    ) -> (bool, BTreeSet<&'a str>, Option<&'a Condition>) {
        let mut had_type_condition = false;
        let mut type_condition_members = BTreeSet::new();
        let mut condition = None;

        while let Some(Segment::TypeCondition(type_names, type_condition)) = path.inner.get(*idx) {
            had_type_condition = true;
            type_condition_members.extend(type_names.iter().map(|s| s.as_str()));
            condition = condition.or(type_condition.as_ref());
            *idx += 1;
        }

        (had_type_condition, type_condition_members, condition)
    }

    #[allow(clippy::too_many_arguments)]
    fn merged_type_condition_segment(
        normalized_path_key: u64,
        non_type_condition_position: usize,
        type_condition_changed: bool,
        me_had_type_condition: bool,
        other_had_type_condition: bool,
        merged_type_condition_members: BTreeSet<&str>,
        condition: Option<&Condition>,
        possible_types_by_position: &HashMap<(u64, usize), &HashSet<String>>,
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
                Rc::new(
                    merged_type_condition_members
                        .iter()
                        .map(|s| s.to_string())
                        .collect(),
                ),
                condition.cloned(),
            ));
        }

        // Strip the type condition only when the merged types are every type the value there
        // can be. What the siblings happen to ask for isn't enough: with `Animal` being
        // `Cat | Dog | Bird`, dropping `|[Cat|Dog]` would send birds too.
        let possible_types =
            possible_types_by_position.get(&(normalized_path_key, non_type_condition_position));
        if condition.is_none()
            && possible_types.is_some_and(|possible_types| {
                possible_types
                    .iter()
                    .all(|type_name| merged_type_condition_members.contains(type_name.as_str()))
            })
        {
            return None;
        }

        Some(Segment::TypeCondition(
            Rc::new(
                merged_type_condition_members
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
            ),
            condition.cloned(),
        ))
    }

    // If non-type-condition parts differ, do not merge.
    // Example: a.@.b vs a.c.b.
    if !me.eq_without_type_castings(other) {
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
        // Both have the same `@include`/`@skip` there, see `can_be_batched_with`.
        let (me_had_type_condition, me_type_condition_members, condition) =
            consume_type_conditions(me, &mut me_idx);
        let (other_had_type_condition, mut other_type_condition_members, _) =
            consume_type_conditions(other, &mut other_idx);
        let type_condition_changed = me_type_condition_members != other_type_condition_members;

        // Combine type-condition type names from both sides (for example Book + User).
        let mut merged_type_condition_members = me_type_condition_members;
        merged_type_condition_members.append(&mut other_type_condition_members);

        // Keep or remove type condition here based on merged and possible types.
        if let Some(merged_type_condition_segment) = merged_type_condition_segment(
            normalized_path_key,
            non_type_condition_position,
            type_condition_changed,
            me_had_type_condition,
            other_had_type_condition,
            merged_type_condition_members,
            condition,
            possible_types_by_position,
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

/// Hash the response path while ignoring type-condition members, so sibling type branches group.
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

/// Keep candidates with different type-condition positions or directives in separate groups.
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

impl FetchStepData<MultiTypeFetchStep> {
    pub fn can_be_batched_with(&self, other: &Self, supergraph: &SupergraphState) -> bool {
        if !self.can_merge_context(other) {
            return false;
        }

        // merged type branches still declare every variable, even when an entity's ancestor
        // only supplies one branch's context. keep non-null contexts in separate calls.
        if self
            .context_variable_definitions
            .iter()
            .chain(&other.context_variable_definitions)
            .any(|variable| matches!(variable.variable_type, TypeNode::NonNull(_)))
        {
            return false;
        }

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
        if !self
            .response_path
            .eq_without_type_castings(&other.response_path)
        {
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

        // One slot with a type condition in a row. More than one means "both", which merging
        // them as a union would get wrong.
        let has_stacked_type_conditions = |path: &MergePath| {
            path.inner.windows(2).any(|pair| {
                matches!(
                    pair,
                    [Segment::TypeCondition(..), Segment::TypeCondition(..)]
                )
            })
        };
        if has_stacked_type_conditions(&self.response_path)
            || has_stacked_type_conditions(&other.response_path)
        {
            return false;
        }

        // The types may differ at one slot only. `[A].child[X]` and `[B].child[Y]` would become
        // `[A|B].child[X|Y]`, which also reads the Y children of A and the X children of B.
        let slots_with_other_types = self
            .response_path
            .inner
            .iter()
            .zip(other.response_path.inner.iter())
            .filter(|(left, right)| {
                matches!(
                    (left, right),
                    (Segment::TypeCondition(left, _), Segment::TypeCondition(right, _)) if left != right
                )
            })
            .count();
        if slots_with_other_types > 1 {
            return false;
        }

        if self.has_arguments_conflicts_with(other) {
            return false;
        }

        // Batched, both outputs go in one selection set per type, even though they come from
        // different places in the response. `thumbnail(width: 100)` under `Aquatics.photo` and
        // `thumbnail(width: 200)` under `Reptiles.photo` can't both be `Photo.thumbnail`.
        // `BatchFetch` can still send the two in one request.
        let output_conflicts = FetchStepSelections::<MultiTypeFetchStep>::iter_matching_types(
            &self.output,
            &other.output,
            |_, self_selections, other_selections| {
                response_keys_conflict(self_selections, other_selections)
            },
        );
        if output_conflicts.iter().any(|(_, conflict)| *conflict) {
            return false;
        }

        // Merged, every entity at either path gets the fields of both. `... on Product { a }`
        // under `ComponentA` and `... on Product { b }` under `ComponentB` would ask the
        // `ComponentA` products for `b` too, and the `ComponentB` ones for `a`.
        // Those stay apart, `BatchFetch` still sends them in one request, an alias each.
        // At the same path it's the same entities, so asking for both is fine.
        if self.response_path != other.response_path
            && outputs_differ_for_a_shared_type(&self.output, &other.output, supergraph)
        {
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

/// Whether an object type gets other fields from `left` than from `right`.
/// Abstract types count for every object type they can be.
/// A type only one side asks for is fine. The paths differ at one type condition only, so
/// either that's the entity's own (`pet|[Cat]` and `pet|[Bird]`, the other side never reaches
/// it), or the rest of the path is the same and so are the types both sides end up at.
fn outputs_differ_for_a_shared_type(
    left: &FetchStepSelections<MultiTypeFetchStep>,
    right: &FetchStepSelections<MultiTypeFetchStep>,
    supergraph: &SupergraphState,
) -> bool {
    if left.iter_selections().eq(right.iter_selections()) {
        return false;
    }

    let overlap = |left_type: &str, right_type: &str| {
        left_type == right_type
            || match (
                supergraph.all_possible_types(left_type),
                supergraph.all_possible_types(right_type),
            ) {
                (Some(left), Some(right)) => !left.is_disjoint(right),
                (Some(left), None) => left.contains(right_type),
                (None, Some(right)) => right.contains(left_type),
                (None, None) => false,
            }
    };

    left.iter_selections().any(|(left_type, left_selections)| {
        right
            .iter_selections()
            .any(|(right_type, right_selections)| {
                left_selections != right_selections && overlap(left_type, right_type)
            })
    })
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use ahash::{HashMap, HashMapExt, HashSet};

    use crate::query_planner::{
        ast::merge_path::{Condition, FieldPathSegment, MergePath, Segment},
        planner::plan_nodes::FlattenNodePath,
    };

    use super::{merge_batched_response_paths, normalized_path_hash};

    fn path(type_name: &str) -> MergePath {
        path_with_condition(&[type_name], None)
    }

    /// `products.@|[types] condition.reviews.@.product`
    fn path_with_condition(type_names: &[&str], condition: Option<Condition>) -> MergePath {
        MergePath::new(vec![
            Segment::Field(
                FieldPathSegment::named("products".to_string()).into(),
                0,
                None,
            ),
            Segment::List,
            Segment::TypeCondition(
                Rc::new(type_names.iter().map(|name| name.to_string()).collect()),
                condition,
            ),
            Segment::Field(
                FieldPathSegment::named("reviews".to_string()).into(),
                0,
                None,
            ),
            Segment::List,
            Segment::Field(
                FieldPathSegment::named("product".to_string()).into(),
                0,
                None,
            ),
        ])
    }

    /// Both calls are under `... on Book @include(if: $a)`, and so is the merged one.
    /// Without `$a`, it would run when `$a` is false too, without the keys only fetched under it.
    #[test]
    fn keeps_condition_of_type_condition() {
        let condition = Some(Condition::Include("a".to_string()));
        let me = path_with_condition(&["Book"], condition.clone());
        let other = path_with_condition(&["Book"], condition);

        let normalized_path_key = normalized_path_hash(&me);
        let merged =
            merge_batched_response_paths(&me, &other, normalized_path_key, &HashMap::new());

        assert_eq!(merged, me);
    }

    /// `Book` and `User` are every type there, but the type condition has `@include(if: $a)`,
    /// so it stays.
    #[test]
    fn keeps_exhaustive_type_list_with_condition() {
        let condition = Some(Condition::Include("a".to_string()));
        let me = path_with_condition(&["Book"], condition.clone());
        let other = path_with_condition(&["User"], condition.clone());
        let normalized_path_key = normalized_path_hash(&me);
        let possible_types = HashSet::from_iter(["Book".to_string(), "User".to_string()]);
        let possible_types_by_position =
            HashMap::from_iter([((normalized_path_key, 2), &possible_types)]);

        let merged = merge_batched_response_paths(
            &me,
            &other,
            normalized_path_key,
            &possible_types_by_position,
        );

        assert_eq!(merged, path_with_condition(&["Book", "User"], condition));
    }

    #[test]
    fn keeps_non_exhaustive_type_list_in_flatten_path() {
        let me = path("Book");
        let other = path("User");
        let normalized_path_key = normalized_path_hash(&me);
        let possible_types = HashSet::from_iter([
            "Book".to_string(),
            "User".to_string(),
            "Magazine".to_string(),
        ]);
        let possible_types_by_position =
            HashMap::from_iter([((normalized_path_key, 2), &possible_types)]);

        let merged = merge_batched_response_paths(
            &me,
            &other,
            normalized_path_key,
            &possible_types_by_position,
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
        let possible_types = HashSet::from_iter(["Book".to_string(), "User".to_string()]);
        let possible_types_by_position =
            HashMap::from_iter([((normalized_path_key, 2), &possible_types)]);

        let merged = merge_batched_response_paths(
            &me,
            &other,
            normalized_path_key,
            &possible_types_by_position,
        );

        assert_eq!(
            format!("{}", FlattenNodePath::from(merged)),
            "products.@.reviews.@.product"
        );
    }
}
