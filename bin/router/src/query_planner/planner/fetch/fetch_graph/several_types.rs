//! Steps that take the objects of several types while the graph is built.
//!
//! The graph is built with `SingleTypeFetchStep`: a step is made for one type. Except for
//! entity calls at a spot that ends with a type condition on several types, like
//! `search.@|[Book|Movie]`:
//! - a batched call: the calls for each type there go to one step, see `joins_other_types`.
//!   Found again with `batched_call_hash`, as every type there gets the same one.
//! - any other step that takes the types planned with its own, see `planned_with` and
//!   `SelectionAttributes::also`: one for a field with `@requires`, for what such a field needs,
//!   or an entity call whose key takes another fetch and so isn't batched. Found again like any
//!   other step, by the type it was made for, see `SeveralTypes::made_for`.
//!
//! Such a step has an input and an output for each of its types, see `take_type`, and whatever
//! goes in its output says which type it's for, see `FetchStepData::add_output_at`. The single
//! type ones don't work on it: `FetchStepSelections::add` and `first_definition_name` are for
//! one type. `SeveralTypes` keeps track of them, and once the graph is built they're finished
//! like batching would, see `finish_batched_steps`.

use crate::query_planner::ast::merge_path::{Condition, MergePath, Segment};
use crate::query_planner::ast::selection_item::SelectionItem;
use crate::query_planner::ast::selection_set::{
    response_keys_conflict, FieldSelection, SelectionSet,
};
use crate::query_planner::planner::fetch::error::FetchGraphError;
use crate::query_planner::planner::fetch::fetch_step_data::FetchStepFlags;
use crate::query_planner::planner::fetch::state::SingleTypeFetchStep;
use crate::query_planner::state::supergraph_state::SubgraphName;
use petgraph::stable_graph::NodeIndex;
use std::collections::{BTreeSet, HashMap, HashSet};
use tracing::trace;

use super::{create_fetch_step_for_entity_call, hash_without_types, EntityCall, FetchGraph};

/// The steps that take several types, see the module docs. Only kept while the graph is built.
#[derive(Debug, Clone, Default)]
pub(crate) struct SeveralTypes {
    /// Found by `batched_call_hash`.
    batched: HashSet<NodeIndex>,
    /// Found like any other step.
    others: HashSet<NodeIndex>,
    /// The type a step was made for, once it takes others too, see `take_type`. Lookups pass
    /// that type, and it's not always the one that sorts first.
    made_for: HashMap<NodeIndex, String>,
}

impl SeveralTypes {
    /// Only for a step nothing's connected to yet: `connect` files it under its entity call, and
    /// what that is depends on whether it's batched.
    fn add_batched(&mut self, step: NodeIndex) {
        self.batched.insert(step);
    }

    fn add_other(&mut self, step: NodeIndex) {
        if !self.batched.contains(&step) {
            self.others.insert(step);
        }
    }

    pub(super) fn made_for(&self, step: NodeIndex) -> Option<&str> {
        self.made_for.get(&step).map(String::as_str)
    }

    pub(super) fn is_batched(&self, step: NodeIndex) -> bool {
        self.batched.contains(&step)
    }

    pub(super) fn contains(&self, step: NodeIndex) -> bool {
        self.batched.contains(&step) || self.others.contains(&step)
    }

    /// All of them, to finish once the graph is built.
    pub(super) fn into_steps(self) -> impl Iterator<Item = NodeIndex> {
        self.batched.into_iter().chain(self.others)
    }
}

impl FetchGraph<SingleTypeFetchStep> {
    /// The children of `parent` that take the objects at `response_path`, whatever their type,
    /// see `joins_other_types`. Newest first, and they still need checking too.
    fn batched_entity_calls(
        &self,
        parent: NodeIndex,
        subgraph_name: &SubgraphName,
        response_path: &MergePath,
        condition: Option<&Condition>,
    ) -> impl Iterator<Item = NodeIndex> + '_ {
        self.children_making(
            parent,
            batched_call_hash(subgraph_name, response_path, condition),
        )
    }
}

/// Makes `step` take the objects of `type_name` too, when it doesn't yet, see
/// `SelectionAttributes::also`: an input and an output for it.
pub(super) fn take_type(
    fetch_graph: &mut FetchGraph<SingleTypeFetchStep>,
    step_index: NodeIndex,
    type_name: &str,
) -> Result<(), FetchGraphError> {
    let step = fetch_graph.get_step_data(step_index)?;
    if step.input.selections_for_definition(type_name).is_some() {
        return Ok(());
    }
    if !step.input.is_fetching_multiple_types() {
        if let Some(made_for) = step.input.first_definition_name() {
            let made_for = made_for.to_string();
            fetch_graph
                .several_types
                .made_for
                .insert(step_index, made_for);
        }
    }
    let step = fetch_graph.get_step_data_mut(step_index)?;
    step.input.add_definition(type_name);
    step.input.add_to(
        type_name,
        &SelectionSet {
            items: vec![SelectionItem::Field(FieldSelection::new_typename())],
        },
    )?;
    step.output.add_definition(type_name);
    Ok(())
}

/// The types planned with `type_name` at the end of `response_path`, see
/// `SelectionAttributes::also`: the type condition there, when it has several and `type_name`
/// is one of them.
pub(super) fn planned_with<'a>(
    response_path: &'a MergePath,
    type_name: &str,
) -> Option<&'a BTreeSet<String>> {
    match response_path.last() {
        Some(Segment::TypeCondition(type_names, _))
            if type_names.len() > 1 && type_names.contains(type_name) =>
        {
            Some(type_names)
        }
        _ => None,
    }
}

/// Adds `selection_set` to what `step` reads of `type_name`, and of the types planned with it,
/// see `planned_with`. The step takes those types too, and becomes one of the steps for several
/// types, finished like them once the graph is built.
pub(super) fn add_input_for_types(
    fetch_graph: &mut FetchGraph<SingleTypeFetchStep>,
    step: NodeIndex,
    response_path: &MergePath,
    type_name: &str,
    selection_set: &SelectionSet,
) -> Result<(), FetchGraphError> {
    match planned_with(response_path, type_name) {
        Some(type_names) => {
            fetch_graph.several_types.add_other(step);
            for type_name in type_names {
                take_type(fetch_graph, step, type_name)?;
                fetch_graph.add_input(step, type_name, selection_set)?;
            }
            Ok(())
        }
        None => fetch_graph.add_input(step, type_name, selection_set),
    }
}

/// Goes into `batched_call_hash` only.
const BATCHED_CALL: u8 = 2;

/// Like `entity_call_hash`, for an entity call that takes the objects of several types at the
/// end of `response_path`, see `joins_other_types`. Every type there gets the same hash. The
/// path ends with a type condition: `joins_other_types` checks that before a batched step is
/// made or looked for.
pub(super) fn batched_call_hash(
    subgraph_name: &SubgraphName,
    response_path: &MergePath,
    condition: Option<&Condition>,
) -> u64 {
    use std::hash::{Hash, Hasher};

    let mut hasher = ahash::AHasher::default();
    subgraph_name.hash(&mut hasher);
    condition.hash(&mut hasher);
    match response_path.inner.split_last() {
        Some((Segment::TypeCondition(_, type_condition), rest)) => {
            hash_without_types(rest, &mut hasher);
            type_condition.hash(&mut hasher);
        }
        // still a hash, the steps found get their path compared anyway
        _ => debug_assert!(false, "a batched call's path ends with a type condition"),
    }
    // so it's never the hash of a plain call to the same spot
    BATCHED_CALL.hash(&mut hasher);
    hasher.finish()
}

/// Whether an entity call for `input_type_name` at `response_path` can take the objects of
/// other types there too, as one call: the path ends with a type condition on that type, like
/// `search.@|[Book]`, and the call reads and writes that type. The calls for `Movie` and `Song`
/// there go to the same subgraph with their own key and fields, and each type gets its own
/// selection set, so they can't get in each other's way.
pub(super) fn joins_other_types(
    response_path: &MergePath,
    input_type_name: &str,
    output_type_name: &str,
) -> bool {
    if input_type_name != output_type_name {
        return false;
    }
    let Some((Segment::TypeCondition(type_names, _), rest)) = response_path.inner.split_last()
    else {
        return false;
    };
    type_names.contains(input_type_name)
        // two type conditions in a row mean both, a union of them would be wrong
        && !matches!(rest.last(), Some(Segment::TypeCondition(..)))
}

/// Whether two paths are the same but for the types of the type condition at their end.
fn same_but_last_types(left: &MergePath, right: &MergePath) -> bool {
    match (left.inner.split_last(), right.inner.split_last()) {
        (
            Some((Segment::TypeCondition(_, left_condition), left_rest)),
            Some((Segment::TypeCondition(_, right_condition), right_rest)),
        ) => left_condition == right_condition && left_rest == right_rest,
        _ => false,
    }
}

/// The step for an entity call that can take the objects of other types at the same spot too,
/// see `joins_other_types`. One that has the type already, when it was made for the same key,
/// or another key that doesn't clash with it. Otherwise one at that spot that doesn't have the
/// type yet, which takes it on. Or a new one.
pub(super) fn ensure_batched_fetch_step(
    fetch_graph: &mut FetchGraph<SingleTypeFetchStep>,
    call: EntityCall<'_>,
) -> Result<NodeIndex, FetchGraphError> {
    let EntityCall {
        parent: parent_fetch_step_index,
        subgraph_name,
        input_type_name: type_name,
        response_path,
        key,
        conditions,
        created_from_requires,
        ..
    } = call;
    let condition = conditions.current;
    let mut with_type_and_key = None;
    let mut with_type = None;
    let mut without_type = None;
    for child_index in fetch_graph.batched_entity_calls(
        parent_fetch_step_index,
        subgraph_name,
        response_path,
        condition,
    ) {
        let step = fetch_graph.get_step_data(child_index)?;
        if step.service_name != *subgraph_name
            || step.condition.as_ref() != condition
            || step.flags.contains(FetchStepFlags::USED_FOR_REQUIRES) != created_from_requires
            // same rule as a call for one type: planned under the same gates, the merge works
            // out where the gates go when calls under different ones come together
            || step.gated_by != conditions.from_root
            || !same_but_last_types(&step.response_path, response_path)
        {
            continue;
        }
        match step.input.selections_for_definition(type_name) {
            Some(input) => {
                if input.contains(&key.selection_set) {
                    with_type_and_key = Some(child_index);
                    break;
                }
                if with_type.is_none() && !response_keys_conflict(input, &key.selection_set) {
                    with_type = Some(child_index);
                }
            }
            None => {
                without_type = without_type.or(Some(child_index));
            }
        }
    }

    let step_index = match with_type_and_key.or(with_type).or(without_type) {
        Some(step_index) => step_index,
        None => {
            let step_index = create_fetch_step_for_entity_call(
                fetch_graph,
                parent_fetch_step_index,
                subgraph_name,
                type_name,
                type_name,
                response_path,
                conditions,
                created_from_requires,
            );
            fetch_graph.several_types.add_batched(step_index);
            step_index
        }
    };
    // the call's type, and the ones planned with it, see `SelectionAttributes::also`. They
    // read the same key.
    let type_names: Vec<&str> = match response_path.last() {
        Some(Segment::TypeCondition(type_names, _)) => {
            type_names.iter().map(String::as_str).collect()
        }
        _ => vec![type_name],
    };
    for type_name in type_names {
        let new_type = fetch_graph
            .get_step_data(step_index)?
            .input
            .selections_for_definition(type_name)
            .is_none();
        if new_type {
            trace!(
                "fetch step [{}] takes the {} objects at {} too",
                step_index.index(),
                type_name,
                response_path
            );
            take_type(fetch_graph, step_index, type_name)?;
            let step = fetch_graph.get_step_data_mut(step_index)?;
            let last = step.response_path.len().saturating_sub(1);
            if let Some(path) = step.response_path.with_type_added_at(last, type_name) {
                step.response_path = path;
            }
        }
        fetch_graph.add_input(step_index, type_name, &key.selection_set)?;
    }
    Ok(step_index)
}
