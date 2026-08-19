use ahash::HashMap as AHashMap;
use bytes::BufMut;
use hive_router_query_planner::planner::response_shape::TYPENAME_SLOT;
use hive_router_query_planner::planner::slot_path::{RequiresStep, SlotRewrite};
use std::collections::hash_map::Entry;
use xxhash_rust::xxh3::xxh3_64;

use crate::execution::rewrites::SlotRewriteExt;

use crate::{
    introspection::schema::PossibleTypes,
    json_writer::{write_and_escape_string, write_f64, write_i64, write_u64},
    response::value::Value,
    utils::consts::{
        CLOSE_BRACE, CLOSE_BRACKET, COLON, COMMA, FALSE, NULL, OPEN_BRACE, OPEN_BRACKET, QUOTE,
        TRUE, TYPENAME_FIELD_NAME, TYPENAME_JSON_FIELD,
    },
};

/// Projects one entity into the `representations` array, deduplicating on the bytes it
/// produced, and returns the hash to record for this entity's position.
///
/// Deduplicating on the projected bytes — rather than walking the `requires` selection set a
/// second time to hash the tree — removes a whole traversal per entity, along with its
/// `binary_search` per field. It is also more precise: two entities that differ only in
/// fields `requires` does not select now collapse into one representation.
///
/// The trade: a duplicate now has to be projected before it can be recognised as one,
/// where the tree hash could reject it first. So the cost here is flat in the duplicate
/// ratio, ~83-105us per 1000 entities, while the old cost scaled with how many were
/// unique. Measured over 1000 entities:
///
/// | distinct | before   | after   |
/// |----------|---------:|--------:|
/// | 1000     | 266.7 us |  99.5 us|
/// | 50       |  75.9 us |  83.1 us|
///
/// A 2.7x win when entities are mostly unique, against ~9% when almost all are duplicates.
///
/// `None` means nothing was written for this position: either the entity was null, or
/// projection produced no fields. Both are treated the same way at merge time.
#[allow(clippy::too_many_arguments)]
pub fn push_representation(
    possible_types: &PossibleTypes,
    requires: &[RequiresStep],
    input_rewrites: &[SlotRewrite],
    arena: &bumpalo::Bump,
    entity: &Value<'_>,
    buffer: &mut Vec<u8>,
    seen: &mut AHashMap<u64, usize>,
    next_index: &mut usize,
) -> Option<u64> {
    if entity.is_null() {
        return None;
    }

    // Rewrites have to run before hashing, because they change the bytes that get sent.
    // That means a duplicate entity is now cloned and rewritten before being discarded,
    // where before it was discarded first — only when `input_rewrites` is set, which is
    // rare, and it buys dedup on the post-rewrite value.
    let rewritten;
    let entity = if input_rewrites.is_empty() {
        entity
    } else {
        rewritten = arena.alloc(entity.clone());
        for rewrite in input_rewrites {
            rewrite.rewrite(possible_types, rewritten);
        }
        &*rewritten
    };

    // The separator is written before the entry and rolled back with it, so it never ends
    // up inside the hashed bytes — otherwise the same entity would hash differently
    // depending on its position in the array.
    let entry_start = buffer.len();
    if *next_index > 0 {
        buffer.put(COMMA);
    }
    let bytes_start = buffer.len();

    if !project_requires(possible_types, requires, entity, buffer, true, None) {
        buffer.truncate(entry_start);
        return None;
    }

    let hash = xxh3_64(&buffer[bytes_start..]);
    match seen.entry(hash) {
        Entry::Occupied(_) => buffer.truncate(entry_start),
        Entry::Vacant(vacant) => {
            vacant.insert(*next_index);
            *next_index += 1;
        }
    }

    Some(hash)
}

fn write_response_key(first: bool, response_key: Option<&str>, buffer: &mut Vec<u8>) {
    if !first {
        buffer.put(COMMA);
    }
    if let Some(response_key) = response_key {
        buffer.put(QUOTE);
        buffer.put(response_key.as_bytes());
        buffer.put(QUOTE);
        buffer.put(COLON);
    }
}

#[inline]
fn write_typename_field(buffer: &mut Vec<u8>, type_name: &str) {
    buffer.put(TYPENAME_JSON_FIELD);
    write_and_escape_string(buffer, type_name);
}

/// Writes one entity's `representations` entry by running the compiled `requires` program.
///
/// The program resolved every response key to a slot at plan time, so this walks the entity
/// by index — no key comparison, no `binary_search` per field, and no interpreting of
/// `SelectionItem`s.
pub fn project_requires(
    possible_types: &PossibleTypes,
    steps: &[RequiresStep],
    entity: &Value,
    buffer: &mut Vec<u8>,
    first: bool,
    response_key: Option<&str>,
) -> bool {
    match entity {
        // An absent field is left out of the representation entirely; an explicit `null` is
        // handled by the caller, which writes it.
        Value::Absent | Value::Null => return false,
        Value::Bool(b) => {
            write_response_key(first, response_key, buffer);
            buffer.put(if *b { TRUE } else { FALSE });
        }
        Value::F64(n) => {
            write_response_key(first, response_key, buffer);
            write_f64(buffer, *n);
        }
        Value::I64(n) => {
            write_response_key(first, response_key, buffer);
            write_i64(buffer, *n);
        }
        Value::U64(n) => {
            write_response_key(first, response_key, buffer);
            write_u64(buffer, *n);
        }
        Value::String(s) => {
            write_response_key(first, response_key, buffer);
            write_and_escape_string(buffer, s);
        }
        Value::OwnedString(s) => {
            write_response_key(first, response_key, buffer);
            write_and_escape_string(buffer, s);
        }
        Value::RawJson(raw) => {
            write_response_key(first, response_key, buffer);
            buffer.put_slice(raw.as_bytes());
        }
        Value::Array(items) => {
            write_response_key(first, response_key, buffer);
            buffer.put(OPEN_BRACKET);

            let mut first = true;
            for item in items {
                if project_requires(possible_types, steps, item, buffer, first, None) {
                    // Only update `first` if we actually write something
                    first = false;
                }
            }
            buffer.put(CLOSE_BRACKET);
        }
        Value::Object(slots) => {
            if steps.is_empty() || slots.is_empty() {
                return false;
            }

            let parent_first = first;
            let mut first = true;
            project_requires_steps(
                possible_types,
                steps,
                entity,
                buffer,
                &mut first,
                response_key,
                parent_first,
            );
            if first {
                // If no fields were projected, "first" is still true,
                // so we skip writing the closing brace
                return false;
            }
            buffer.put(CLOSE_BRACE);
        }
    };
    true
}

#[inline]
fn is_typename_step(step: &RequiresStep) -> bool {
    matches!(step, RequiresStep::Leaf { key, .. } if key == TYPENAME_FIELD_NAME)
}

#[allow(clippy::too_many_arguments)]
fn project_requires_steps(
    possible_types: &PossibleTypes,
    steps: &[RequiresStep],
    entity: &Value<'_>,
    buffer: &mut Vec<u8>,
    first: &mut bool,
    parent_response_key: Option<&str>,
    parent_first: bool,
) {
    // Read `__typename` up front; it is reserved at slot 0 of every client-visible position.
    let type_name = entity.slot(TYPENAME_SLOT).and_then(Value::as_str);

    // An indicator that only `__typename` is used for the key fields.
    // This is an edge case that we need to identify, in order to detect when
    // `__typename` alone is a valid key but other fields are also required
    if steps.len() == 1 && is_typename_step(&steps[0]) {
        if let Some(type_name) = type_name {
            write_response_key(parent_first, parent_response_key, buffer);
            buffer.put(OPEN_BRACE);
            write_typename_field(buffer, type_name);
            *first = false;

            return;
        }
    }

    for step in steps {
        match step {
            RequiresStep::OnType {
                typename_slot,
                type_condition,
                steps,
            } => {
                let type_name = typename_slot
                    .and_then(|slot| entity.slot(slot))
                    .and_then(Value::as_str)
                    .unwrap_or(type_condition);

                // For projection, both sides of the condition are valid
                if possible_types.entity_satisfies_type_condition(type_name, type_condition)
                    || possible_types.entity_satisfies_type_condition(type_condition, type_name)
                {
                    project_requires_steps(
                        possible_types,
                        steps,
                        entity,
                        buffer,
                        first,
                        parent_response_key,
                        parent_first,
                    );
                }
            }
            _ if is_typename_step(step) => continue,
            RequiresStep::Leaf { key, slot } | RequiresStep::Enter { key, slot, .. } => {
                let original = entity.slot_or_absent(*slot);
                // A field no response ever filled in is left out of the representation. An
                // explicit `null` below is kept, because the subgraph may key on it.
                if original.is_absent() {
                    continue;
                }

                // In most requests, required fields are present and projection succeeds.
                // If projection ends up writing nothing, we rewind to this offset.
                let mut object_start_offset = None;

                if *first {
                    object_start_offset = Some(buffer.len());
                    write_response_key(parent_first, parent_response_key, buffer);
                    buffer.put(OPEN_BRACE);
                    // Write __typename only if the object has other fields,
                    // and if it wasn't written before (first=true)
                    if let Some(type_name) = type_name {
                        write_typename_field(buffer, type_name);
                        *first = false;
                    }
                }

                if original.is_null() {
                    // The field exists and is null, so keep it in the representation.
                    write_response_key(*first, Some(key.as_str()), buffer);
                    buffer.put(NULL);
                    *first = false;
                    continue;
                }

                let nested: &[RequiresStep] = match step {
                    RequiresStep::Enter { steps, .. } => steps,
                    _ => &[],
                };

                let projected = project_requires(
                    possible_types,
                    nested,
                    original,
                    buffer,
                    *first,
                    Some(key.as_str()),
                );

                if projected {
                    *first = false;
                } else if *first {
                    // We opened '{' but produced no field output.
                    // Roll back to keep valid JSON and avoid malformed '{...'.
                    if let Some(offset) = object_start_offset {
                        buffer.truncate(offset);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::project_requires;
    use crate::introspection::schema::PossibleTypes;
    use graphql_tools::parser::query;
    use hive_router_query_planner::ast::{
        selection_set::SelectionSet,
    };
    use hive_router_query_planner::planner::merged_shape::response_shape_for_selections;
    use hive_router_query_planner::planner::slot_path::compile_requires;
    use hive_router_query_planner::utils::parsing::parse_operation;
    use sonic_rs::json;

    use crate::response::subgraph_response::SubgraphResponse;

    fn requires_from_str(requires: &str) -> SelectionSet {
        let operation = parse_operation(&format!("query {{ {requires} }}"));

        let selection_set = operation
            .definitions
            .into_iter()
            .find_map(|def| {
                let query::Definition::Operation(op) = def else {
                    return None;
                };

                match op {
                    query::OperationDefinition::SelectionSet(sel) => Some(sel),
                    query::OperationDefinition::Query(q) => Some(q.selection_set),
                    query::OperationDefinition::Mutation(m) => Some(m.selection_set),
                    query::OperationDefinition::Subscription(s) => Some(s.selection_set),
                }
            })
            .expect("operation must contain a selection set");

        selection_set.into()
    }

    fn project_requires_pretty(requires: &str, entity_json: sonic_rs::Value) -> Option<String> {
        // The entity is parsed against the same shape the `requires` program is compiled
        // from, which is what pairs its slots with the program's.
        let selections = requires_from_str(requires);
        let shape = response_shape_for_selections(&selections);
        let compiled = compile_requires(&selections, &shape);

        let owned = SubgraphResponse::parse_data_with_shape(
            &sonic_rs::to_string(&entity_json).unwrap(),
            &shape,
        );

        let mut buffer = Vec::new();
        let projected = project_requires(
            &PossibleTypes::default(),
            &compiled,
            &owned.data,
            &mut buffer,
            true,
            None,
        );

        if !projected {
            return None;
        }

        let json: sonic_rs::Value = sonic_rs::from_slice(&buffer).unwrap();
        Some(sonic_rs::to_string_pretty(&json).unwrap())
    }

    #[test]
    fn project_requires_variants() {
        insta::assert_snapshot!(
          &project_requires_pretty(
            "contactOptions id",
            json!({
                "__typename": "Ad",
                "contactOptions": null,
                "id": "1"
            }),
          )
          .expect("projection should produce output"),
          @r#"
          {
            "__typename": "Ad",
            "contactOptions": null,
            "id": "1"
          }
        "#);

        insta::assert_snapshot!(
          &project_requires_pretty(
            "id contactOptions",
            json!({
                "__typename": "Ad",
                "contactOptions": null,
                "id": "1"
            }),
          ).expect("projection should produce output"),
          // Fields come out in `requires` selection order, which is what the compiled
          // program walks.
          @r#"
          {
            "__typename": "Ad",
            "id": "1",
            "contactOptions": null
          }
        "#);

        insta::assert_snapshot!(
          &project_requires_pretty(
              "contactOptions id",
              json!({
                  "__typename": "Ad",
                  "id": "1"
              }),
          )
          .expect("projection should produce output"),
          @r#"
          {
            "__typename": "Ad",
            "id": "1"
          }
        "#);

        insta::assert_snapshot!(
          &project_requires_pretty(
              "branch { contactOptions { email } } id",
              json!({
                  "__typename": "Ad",
                  "branch": {
                      "contactOptions": {}
                  },
                  "id": "1"
              }),
          )
          .expect("projection should produce output"),
          @r#"
          {
            "__typename": "Ad",
            "id": "1"
          }
        "#);

        insta::assert_snapshot!(
          &project_requires_pretty(
              "branch { contactOptions { email user { id name } } } id",
              json!({
                  "__typename": "Ad",
                  "branch": {
                      "__typename": "Branch",
                      "contactOptions": null
                  },
                  "id": "1"
              }),
          )
          .expect("projection should produce output"),
          @r#"
          {
            "__typename": "Ad",
            "branch": {
              "__typename": "Branch",
              "contactOptions": null
            },
            "id": "1"
          }
        "#);

        let pretty = project_requires_pretty("contactOptions", json!({}));
        assert_eq!(pretty, None);
    }

    /// Regression testt for https://github.com/graphql-hive/router/issues/1099:
    /// a key that has only `__typename` must still produce a
    /// representation, and using `__typename` alongside another field (in
    /// either order) must not duplicate it, or drop any of the field/__typename
    #[test]
    fn project_requires_typename_key() {
        // Only `__typename` in the key — must still build the representation and return a valid JSON
        insta::assert_snapshot!(
          &project_requires_pretty(
              "__typename", // @key(fields: ["__typename"])
              json!({
                  "__typename": "CatalogEntry",
                  "sku": "SKU-REPRO-001"
              }),
          )
          .expect("projection should produce output"),
          @r#"
          {
            "__typename": "CatalogEntry"
          }
        "#);

        // `__typename` is first, then another field
        insta::assert_snapshot!(
          &project_requires_pretty(
              "__typename id", // @key(fields: ["__typename", "id"])
              json!({
                  "__typename": "CatalogEntry",
                  "id": "1"
              }),
          )
          .expect("projection should produce output"),
          @r#"
          {
            "__typename": "CatalogEntry",
            "id": "1"
          }
        "#);

        // Another field listed first, then `__typename` — same result, no duplicates
        insta::assert_snapshot!(
          &project_requires_pretty(
              "id __typename", // @key(fields: ["id", "__typename"])
              json!({
                  "__typename": "CatalogEntry",
                  "id": "1"
              }),
          )
          .expect("projection should produce output"),
          @r#"
          {
            "__typename": "CatalogEntry",
            "id": "1"
          }
        "#);
    }
}
