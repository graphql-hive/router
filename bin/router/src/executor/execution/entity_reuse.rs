//! Copies entities that are already in the response, instead of asking a subgraph for them again.
//!
//! The planner tells an entity fetch where earlier fetches put objects of the same type with
//! the same fields (`EntityReuse`). Here we hash those objects the same way the fetch hashes its
//! representations. When a representation matches one, it's the same entity with the same
//! `@requires` inputs, so we copy the fields and leave it out of the request.

use crate::{
    executor::{
        execution::context::{EntityRef, Known, TargetEntities},
        introspection::schema::{PossibleTypes, SchemaMetadata},
        response::{merge::EntityMerger, value::Value},
        utils::{
            prehashed_map::PrehashedMap,
            traverse::{traverse_and_callback, traverse_and_callback_mut},
        },
    },
    query_planner::{
        ast::requires::{RequiresSelection, RequiresSelectionSetRef},
        planner::plan_nodes::{EntityReuse, ResponsePathRef},
    },
};

/// What one fetch could copy instead of sending. `seen` starts out with the objects already in
/// the response, by the hash their representation has, and `collect_context_entities` takes it
/// over as its table of every entity it knows.
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
        for source in &reuse.sources {
            traverse_and_callback(data, source.as_slice(), possible_types, &mut |object, _| {
                if object.is_object() && holds_every_field(object, required_selections) {
                    seen.entry(object.to_hash(required_selections, possible_types))
                        .or_insert(Known::InResponse(object));
                }
            });
        }
        Self {
            seen,
            copy: reuse.copy.root_selections(),
            copies: Vec::new(),
        }
    }

    /// Copies the fields the fetch asks for from `source` and says where the copy sits in
    /// [`Self::into_copies`]. None if it doesn't have all of them.
    pub(super) fn copy(&mut self, source: &Value<'v>) -> Option<u32> {
        self.copies.push(copy_object(source, self.copy)?);
        Some(self.copies.len() as u32 - 1)
    }

    pub fn into_copies(self) -> Vec<Value<'v>> {
        self.copies
    }
}

/// The entities one fetch copied, and the paths of its targets. The targets that are
/// [`EntityRef::Copied`] get them.
pub struct Copies<'a, 'v> {
    pub paths: Vec<(ResponsePathRef<'a>, TargetEntities)>,
    pub copies: Vec<Value<'v>>,
}

/// Puts the copies a wave made into their targets, before its requests are awaited. Nothing was
/// sent for them, so there are no errors or rewrites to apply.
pub fn merge_copies<'v>(
    data: &mut Value<'v>,
    copies: Vec<Copies<'_, 'v>>,
    schema_metadata: &SchemaMetadata,
) {
    let copied = |target: &Option<EntityRef>| match target {
        Some(EntityRef::Copied { index }) => Some(*index as usize),
        _ => None,
    };
    for Copies { paths, mut copies } in copies {
        let mut merger = EntityMerger::new(
            &mut copies,
            paths
                .iter()
                .flat_map(|(_, targets)| targets.iter())
                .filter_map(copied),
        );
        for (path, targets) in &paths {
            let mut targets = targets.iter();
            traverse_and_callback_mut(
                data,
                path.as_slice(),
                schema_metadata,
                None,
                &mut |target, _| {
                    if let Some(index) = targets.next().and_then(copied) {
                        merger.merge_into(target, index);
                    }
                },
            );
        }
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

/// Copies the selected fields of `source`. None if any is missing, or doesn't have the shape
/// we expect, and then the subgraph answers for that entity as usual.
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
            // The planner only gives us one, around the whole entity.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query_planner::{
        ast::requires::RequiresSelectionSet, utils::parsing::parse_operation,
    };
    use serde::Deserialize;

    fn parse(json: &'static str) -> Value<'static> {
        let mut deserializer = sonic_rs::Deserializer::from_slice(json.as_bytes());
        Value::deserialize(&mut deserializer).unwrap()
    }

    fn selection(query: &str) -> RequiresSelectionSet {
        use graphql_tools::parser::query;
        let document = parse_operation(query);
        let selection = document
            .definitions
            .iter()
            .find_map(|definition| match definition {
                query::Definition::Operation(query::OperationDefinition::SelectionSet(set)) => {
                    Some(set.clone())
                }
                _ => None,
            })
            .expect("a bare selection set");
        let set: crate::query_planner::ast::selection_set::SelectionSet = selection.into();
        RequiresSelectionSet::from(&set)
    }

    #[test]
    fn copies_the_selected_fields_and_skips_incomplete_entities() {
        let copy = selection("{ ... on Product { __typename name price } }");
        let table = parse(r#"{"__typename": "Product", "upc": "1", "name": "Table", "price": 10}"#);
        assert_eq!(
            copy_object(&table, copy.root_selections())
                .unwrap()
                .to_string(),
            r#"{"__typename": "Product", "name": "Table", "price": 10}"#
        );
        // no name, so the subgraph has to answer for it
        let chair = parse(r#"{"__typename": "Product", "upc": "2", "price": 20}"#);
        assert!(copy_object(&chair, copy.root_selections()).is_none());
    }
}
