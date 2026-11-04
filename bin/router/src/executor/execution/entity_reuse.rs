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

/// What one fetch could copy instead of sending. `collect_context_entities` takes `seen` over as
/// its table of every entity it knows.
pub struct ReuseIndex<'d, 'v> {
    pub(super) seen: PrehashedMap<Known<'d, 'v>>,
    copy: RequiresSelectionSetRef<'d>,
    copies: Vec<Value<'v>>,
}

impl<'d, 'v> ReuseIndex<'d, 'v> {
    pub fn build(
        reuse: &'d EntityReuse,
        data: &'d Value<'v>,
        required_selections: RequiresSelectionSetRef<'_>,
        possible_types: &PossibleTypes,
    ) -> Self {
        let mut seen = PrehashedMap::default();
        let trie = reuse.trie();
        trie.walk(data, possible_types, &mut |_, object, _| {
            if object.is_object() && holds_every_field(object, required_selections) {
                seen.entry(object.to_hash(required_selections, possible_types))
                    .or_insert(Known::InResponse(object));
            }
        });
        Self {
            seen,
            copy: reuse.copy.root_selections(),
            copies: Vec::new(),
        }
    }

    pub(super) fn copy(&mut self, source: &Value<'v>) -> Option<u32> {
        self.copies.push(copy_object(source, self.copy)?);
        Some(self.copies.len() as u32 - 1)
    }

    pub fn into_copies(self) -> Vec<Value<'v>> {
        self.copies
    }
}

/// The entities one fetch copied, and its `paths` among the ones of `trie`, with their targets.
pub struct Copies<'a, 'v> {
    pub trie: &'a PathTrie,
    pub paths: Range<usize>,
    pub targets: Arc<TargetEntities>,
    pub copies: Vec<Value<'v>>,
}

/// Nothing was sent for the copies, so there are no errors or rewrites to apply.
pub fn merge_copies<'v>(
    data: &mut Value<'v>,
    copies: Vec<Copies<'_, 'v>>,
    possible_types: &PossibleTypes,
) {
    let copied = |target: &Option<EntityRef>| match target {
        Some(EntityRef::Copied { index }) => Some(*index as usize),
        _ => None,
    };
    for Copies {
        trie,
        paths,
        targets,
        mut copies,
    } in copies
    {
        let mut merger = EntityMerger::new(&mut copies, targets.iter().filter_map(copied));
        // the next target on each of the fetch's paths
        let mut next: Vec<_> = (0..targets.path_count())
            .map(|path| targets.path(path).iter())
            .collect();
        trie.walk_mut(data, possible_types, None, &mut |node, target, _| {
            for &path in trie.paths_ending_at(node) {
                if !paths.contains(&path) {
                    continue;
                }
                if let Some(index) = next[path - paths.start].next().and_then(copied) {
                    merger.merge_into(target, index);
                }
            }
        });
    }
}

/// Whether `value` has every field of `selections`, none of them null. An entity with a missing
/// or null key can't be told apart from another one like it, so we never copy from it. A null
/// `@requires` field counts too, that only costs a fetch.
fn holds_every_field(value: &Value, selections: RequiresSelectionSetRef<'_>) -> bool {
    match value {
        Value::Null => false,
        Value::Object(fields) => selections.iter().all(|selection| match selection {
            RequiresSelection::Field {
                name,
                alias,
                selections,
                ..
            } => Value::object_get(fields, alias.unwrap_or(name))
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
fn copy_object<'a>(
    source: &Value<'a>,
    selections: RequiresSelectionSetRef<'_>,
) -> Option<Value<'a>> {
    let mut copied = Vec::new();
    // fields nearly always come in the order we ask for them, so each lookup starts where the
    // last one stopped
    copy_fields(source.as_object()?, selections, &mut copied, &mut 0)?;
    Some(Value::Object(copied))
}

/// Pushes the selected fields into `copied`. Inline fragments push into the same list, so a
/// copy is one allocation.
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
