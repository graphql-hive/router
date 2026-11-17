//! Copies entities that are already in the response, instead of asking a subgraph for them again.
//! A representation that matches an object at one of the planner's `EntityReuse` sources is the
//! same entity with the same `@requires` inputs.

use std::{ops::Range, sync::Arc};

use crate::{
    executor::{
        execution::context::{EntityRef, Known, TargetEntities},
        introspection::schema::PossibleTypes,
        response::{merge::EntityMerger, value::Value},
        utils::{prehashed_map::PrehashedMap, traverse::PathTrie},
    },
    query_planner::{
        ast::requires::{RequiresSelection, RequiresSelectionSetRef},
        planner::plan_nodes::EntityReuse,
    },
};

/// Puts every object at the planner's sources that has all the fields of a representation into
/// `seen`, by the same hash a representation of it would get.
///
/// A target whose representation hashes the same gets a copy without comparing fields, trusting
/// the 64-bit hash like deduplicating representations does.
pub(super) fn index_sources<'d, 'v>(
    reuse: &EntityReuse,
    data: &'d Value<'v>,
    required_selections: RequiresSelectionSetRef<'_>,
    possible_types: &PossibleTypes,
    seen: &mut PrehashedMap<Known<'d, 'v>>,
) {
    let trie = reuse.trie();
    trie.walk(data, possible_types, &mut |_, object, _| {
        if object.is_object() && holds_every_field(object, required_selections) {
            seen.entry(object.to_hash(required_selections, possible_types))
                .or_insert(Known::InResponse(object));
        }
    });
}

/// What one fetch (or batch alias) copied: its `paths` among the ones of `trie`, which entity
/// goes into each target at their ends, and the copied entities those point at.
pub struct EntityCopyBatch<'a, 'v> {
    pub trie: &'a PathTrie,
    pub paths: Range<usize>,
    pub targets_by_path: Arc<TargetEntities>,
    pub entities: Vec<Value<'v>>,
}

/// Which copied entity goes into `target`, if it's one of the copied ones
fn copied_index(target: &Option<EntityRef>) -> Option<usize> {
    match target {
        Some(EntityRef::Copied { index }) => Some(*index as usize),
        _ => None,
    }
}

/// Nothing was sent for the copies, so there are no errors or rewrites to apply.
pub fn merge_copies<'v>(
    data: &mut Value<'v>,
    mut batches: Vec<EntityCopyBatch<'_, 'v>>,
    possible_types: &PossibleTypes,
) {
    // the aliases of a batch fetch come one after another and share its trie, so one walk merges
    // all of them
    for group in batches.chunk_by_mut(|a, b| std::ptr::eq(a.trie, b.trie)) {
        merge_walk(data, group, possible_types);
    }
}

fn merge_walk<'v>(
    data: &mut Value<'v>,
    group: &mut [EntityCopyBatch<'_, 'v>],
    possible_types: &PossibleTypes,
) {
    let trie = group[0].trie;
    let mut mergers = Vec::with_capacity(group.len());
    // for every path of the trie that copies: its batch, and the slots of its targets still to
    // come, in walk order
    let mut target_slots = vec![None; trie.ends().len()];
    for (batch_index, batch) in group.iter_mut().enumerate() {
        let EntityCopyBatch {
            paths,
            targets_by_path,
            entities,
            ..
        } = batch;
        for (path, trie_path) in paths.clone().enumerate() {
            target_slots[trie_path] = Some((batch_index, targets_by_path.path(path).iter()));
        }
        // the merger only needs the copied ones, to know which entity is used where
        let used = targets_by_path.iter().filter_map(copied_index);
        mergers.push(EntityMerger::new(entities, used));
    }
    trie.walk_mut(data, possible_types, None, &mut |node, target, _| {
        for &path in trie.paths_ending_at(node) {
            let Some((batch_index, slots)) = &mut target_slots[path as usize] else {
                continue;
            };
            // every target the walk visits takes a slot, a `None` or `Fetched` one too. The
            // slots were collected walking these same targets, so skipping one would put every
            // later copy into the wrong object.
            if let Some(index) = slots.next().and_then(copied_index) {
                mergers[*batch_index].merge_into(target, index);
            }
        }
    });
}

/// Whether `value` has every field of `selections`, none of them null. An entity with a missing
/// or null key can't be told apart from another one like it, so we never copy from it. A null
/// `@requires` field counts too, that only costs a fetch.
///
/// A field is read under its name, like the hash and the representation do: in
/// `price: _internal_qp_alias_0` the fetch wrote the value under `_internal_qp_alias_0`, and
/// `price` is only what it's called in the representation.
fn holds_every_field(value: &Value, selections: RequiresSelectionSetRef<'_>) -> bool {
    match value {
        Value::Null => false,
        Value::Object(fields) => selections.iter().all(|selection| match selection {
            RequiresSelection::Field {
                name, selections, ..
            } => Value::object_get(fields, name)
                .is_some_and(|field| holds_every_field(field, selections)),
            RequiresSelection::InlineFragment { selections, .. } => {
                holds_every_field(value, selections)
            }
            RequiresSelection::FragmentSpread(_) => false,
        }),
        Value::Array(items) => items.iter().all(|item| holds_every_field(item, selections)),
        _ => selections.is_empty(),
    }
}

/// Copies the selected fields of `source`. None if any is missing, then the subgraph answers.
pub(super) fn copy_object<'a>(
    source: &Value<'a>,
    selections: RequiresSelectionSetRef<'_>,
) -> Option<Value<'a>> {
    let mut copied = Vec::new();
    // fields nearly always come in the order we ask for them, so each lookup starts where the
    // last one stopped
    copy_fields(source.as_object()?, selections, &mut copied, &mut 0)?;
    Some(Value::Object(copied))
}

/// Pushes the selected fields into `copied`. Inline fragments push into the same object.
/// Their type condition isn't checked: the only fragment the planner puts in a copy is its own
/// `... on Type` around plain fields, see `EntityReuse::copy`.
fn copy_fields<'a>(
    fields: &[(&'a str, Value<'a>)],
    selections: RequiresSelectionSetRef<'_>,
    copied: &mut Vec<(&'a str, Value<'a>)>,
    cursor: &mut usize,
) -> Option<()> {
    copied.reserve(selections.len());
    for selection in selections.iter() {
        match selection {
            RequiresSelection::InlineFragment { selections, .. } => {
                copy_fields(fields, selections, copied, cursor)?
            }
            RequiresSelection::Field {
                name,
                alias,
                selections,
                ..
            } => {
                let position = Value::object_position_from(fields, alias.unwrap_or(name), cursor)?;
                let (key, value) = &fields[position];
                let value = if selections.is_empty() {
                    value.clone()
                } else {
                    copy_nested(value, selections)?
                };
                copied.push((*key, value));
            }
            RequiresSelection::FragmentSpread(_) => return None,
        }
    }
    Some(())
}

fn copy_nested<'a>(
    value: &Value<'a>,
    selections: RequiresSelectionSetRef<'_>,
) -> Option<Value<'a>> {
    match value {
        Value::Null => Some(Value::Null),
        Value::Array(items) => items
            .iter()
            .map(|item| copy_nested(item, selections))
            .collect::<Option<Vec<_>>>()
            .map(Value::Array),
        Value::Object(_) => copy_object(value, selections),
        _ => None,
    }
}
