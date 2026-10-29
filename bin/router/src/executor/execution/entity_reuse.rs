//! Copies entities that are already in the response, instead of asking a subgraph for them again.
//!
//! The planner tells an entity fetch where earlier fetches put objects of the same type with
//! the same fields (`EntityReuse`). Here we hash those objects the same way the fetch hashes its
//! representations. When a representation matches one, it's the same entity with the same
//! `@requires` inputs, so we copy the fields and leave it out of the request.

use std::sync::LazyLock;

use ahash::HashMap as AHashMap;
use ahash::HashMapExt;

use crate::{
    executor::{
        introspection::schema::PossibleTypes, response::value::Value, utils::traverse::PathTrie,
    },
    query_planner::{
        ast::requires::{RequiresSelection, RequiresSelectionSetRef},
        planner::plan_nodes::EntityReuse,
    },
};

/// On unless `HIVE_ROUTER_EXPERIMENTAL_ENTITY_REUSE` is `0` or `false`.
pub static ENTITY_REUSE: LazyLock<bool> = LazyLock::new(|| {
    !std::env::var("HIVE_ROUTER_EXPERIMENTAL_ENTITY_REUSE")
        .is_ok_and(|v| v == "0" || v.eq_ignore_ascii_case("false"))
});

/// Entities in the response one fetch could copy, by the hash of their representation.
pub struct ReuseIndex<'d, 'a> {
    by_hash: AHashMap<u64, &'d Value<'a>>,
    copy: RequiresSelectionSetRef<'d>,
}

impl<'d, 'a> ReuseIndex<'d, 'a> {
    pub fn build(
        reuse: &'d EntityReuse,
        data: &'d Value<'a>,
        required_selections: RequiresSelectionSetRef<'_>,
        possible_types: &PossibleTypes,
    ) -> Self {
        let mut by_hash = AHashMap::new();
        let (trie, _) = PathTrie::new(reuse.sources.iter().map(|source| source.as_slice()));
        trie.walk(data, possible_types, &mut |_, object, _| {
            if object.is_object() {
                by_hash
                    .entry(object.to_hash(required_selections, possible_types))
                    .or_insert(object);
            }
        });
        Self {
            by_hash,
            copy: reuse.copy.root_selections(),
        }
    }

    /// A copy of the fields the fetch asks for, if the entity is here and has all of them.
    pub fn copy_for(&self, hash: u64) -> Option<Value<'a>> {
        copy_object(self.by_hash.get(&hash)?, self.copy)
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

/// A copied entity's index has this bit set, because where copies start (after the fetched
/// ones) is only known once all targets were visited. The rest is its place among the copies.
pub const REUSED_ENTITY: usize = 1 << (usize::BITS - 1);

/// Lines up the subgraph's entities with the indexes the targets point at, and puts the copies
/// after them. A subgraph that sends too few or too many can't shift the copies.
pub fn append_reused<'a>(
    entities: &mut Vec<Value<'a>>,
    fetched_count: usize,
    reused: Vec<Value<'a>>,
) {
    if reused.is_empty() {
        return;
    }
    entities.resize(fetched_count, Value::Null);
    entities.extend(reused);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query_planner::{
        ast::requires::RequiresSelectionSet, planner::plan_nodes::FlattenNodePath,
        planner::plan_nodes::PathSegment, utils::parsing::parse_operation,
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
    fn copies_a_matching_entity_and_skips_incomplete_ones() {
        let data = parse(
            r#"{"topProducts": [
                {"__typename": "Product", "upc": "1", "name": "Table", "price": 10},
                {"__typename": "Product", "upc": "2", "price": 20}
            ]}"#,
        );
        let reuse = EntityReuse {
            sources: vec![FlattenNodePath::from(vec![
                PathSegment::Field("topProducts".into()),
                PathSegment::List,
            ])],
            copy: selection("{ ... on Product { __typename name price } }"),
        };
        let requires = selection("{ ... on Product { __typename upc } }");
        let possible_types = PossibleTypes::default();
        let index = ReuseIndex::build(&reuse, &data, requires.root_selections(), &possible_types);

        let wanted =
            |json: &'static str| parse(json).to_hash(requires.root_selections(), &possible_types);
        let copied = index
            .copy_for(wanted(r#"{"__typename": "Product", "upc": "1"}"#))
            .expect("product 1 is there with every field");
        assert_eq!(
            copied.to_string(),
            r#"{"__typename": "Product", "name": "Table", "price": 10}"#
        );
        // Product 2 has no name, so the subgraph has to answer for it.
        assert!(index
            .copy_for(wanted(r#"{"__typename": "Product", "upc": "2"}"#))
            .is_none());
        // Product 3 isn't in the response at all.
        assert!(index
            .copy_for(wanted(r#"{"__typename": "Product", "upc": "3"}"#))
            .is_none());
    }

    #[test]
    fn copies_go_after_what_the_subgraph_returns() {
        // The subgraph sent one entity instead of two: the copies still land at 2 and 3.
        let mut entities = vec![parse(r#"{"a": 1}"#)];
        append_reused(
            &mut entities,
            2,
            vec![parse(r#"{"b": 2}"#), parse(r#"{"c": 3}"#)],
        );
        assert!(entities[1].is_null());
        assert_eq!(entities[2].to_string(), r#"{"b": 2}"#);
        assert_eq!(entities[3].to_string(), r#"{"c": 3}"#);
    }
}
