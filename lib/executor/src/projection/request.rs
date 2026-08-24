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

/// Projects one entity into the `representations` array, deduplicating it, and returns the
/// key to record for this entity's position.
///
/// Two ways of recognising a repeat, and the cheap one covers the fetches a query plan
/// actually produces:
///
/// **By value.** A flat `requires` program — leaves and type conditions over one object, no
/// nested `Enter` — writes a pure function of `__typename` plus one value per slot, so those
/// values identify the entry before anything is written. A repeat costs one map lookup and no
/// bytes at all. That is what a deeply nested query needs: in
/// `reviews { product { reviews { author { … } } } }` the same handful of products and users
/// are asked for over and over, and the `requires`-projection symbols fall from 8.7% to 1.6%
/// of on-CPU time in a load profile.
///
/// **By projected bytes.** Everything else projects first and hashes what it wrote, then
/// rolls the buffer back if those bytes were already there. Deduplicating on the bytes rather
/// than walking the `requires` selection set a second time to hash the tree is what made this
/// affordable in the first place — it removed a whole traversal per entity, and it is more
/// precise, since two entities differing only in fields `requires` does not select collapse
/// into one representation.
///
/// Over 1000 entities, tree hash → bytes → value key:
///
/// | distinct    | tree hash | bytes   | value key |
/// |-------------|----------:|--------:|----------:|
/// | all 1000    |  266.7 us | 62.2 us |   72.1 us |
/// | half        |         — | 51.7 us |   44.6 us |
/// | few         |         — | 42.7 us |   19.7 us |
/// | almost none |   75.9 us | 40.8 us |   17.2 us |
///
/// The first row is the cost of the value key never paying off: entities that are all
/// distinct are hashed once for nothing before being projected anyway. Entity fetches exist
/// because entities repeat, so that row is the unusual one — but it is a real 16%, and if a
/// workload ever lives there the fix is to stop computing the key, not to compute it better.
///
/// Either way the returned value is only an identifier: it is matched against itself when
/// entities come back, and never interpreted. The two kinds never mix inside one call, since
/// the program is fixed for the whole fetch.
///
/// `None` means nothing was written for this position: either the entity was null, or
/// projection produced no fields. Both are treated the same way at merge time.
#[allow(clippy::too_many_arguments)]
pub fn push_representation<'a: 'scratch, 'scratch>(
    possible_types: &PossibleTypes,
    requires: &[RequiresStep],
    input_rewrites: &[SlotRewrite],
    // Scratch space for the rewritten copy, and only that: nothing allocated here outlives
    // the call, so the call site can pass a plain local arena.
    arena: &'scratch bumpalo::Bump,
    entity: &Value<'a>,
    buffer: &mut Vec<u8>,
    seen: &mut AHashMap<u64, usize>,
    next_index: &mut usize,
) -> Option<u64> {
    if entity.is_null() {
        return None;
    }

    // A rewritten entity is projected from a clone, whose values this key never sees, so the
    // value path is only taken when there are no rewrites.
    if input_rewrites.is_empty() {
        if let Some(key) = flat_requires_key(requires, entity) {
            // One map operation, not a lookup and then an insert: `entry` claims the index
            // for a first sighting, and the rare entity that projects to nothing gives it
            // back below.
            match seen.entry(key) {
                Entry::Occupied(_) => return Some(key),
                Entry::Vacant(vacant) => vacant.insert(*next_index),
            };
            if !project_entry(possible_types, requires, entity, buffer, *next_index) {
                seen.remove(&key);
                return None;
            }
            *next_index += 1;
            return Some(key);
        }
    }

    // Rewrites have to run before hashing, because they change the bytes that get sent.
    // That means a duplicate entity is now copied and rewritten before being discarded, where
    // before it was discarded first — only when `input_rewrites` is set, which is rare, and it
    // buys dedup on the post-rewrite value.
    //
    // The two branches project separately rather than unifying on one `&Value`: the rewritten
    // copy lives in the scratch arena and is a `Value<'scratch>`, which is a different type
    // from the response tree's `Value<'a>`.
    let entry_start = buffer.len();
    let projected = if input_rewrites.is_empty() {
        project_entry(possible_types, requires, entity, buffer, *next_index)
    } else {
        let rewritten = arena.alloc(entity.copy_into(arena));
        for rewrite in input_rewrites {
            rewrite.rewrite(possible_types, rewritten, arena);
        }
        project_entry(possible_types, requires, rewritten, buffer, *next_index)
    };
    if !projected {
        return None;
    }
    // The separator is excluded from the hashed bytes, so the same entity hashes the same way
    // wherever it lands in the array.
    let bytes_start = entry_start + if *next_index > 0 { 1 } else { 0 };

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

/// Writes one `representations` entry, with its leading separator, and rolls both back if the
/// entity projected to nothing.
fn project_entry(
    possible_types: &PossibleTypes,
    requires: &[RequiresStep],
    entity: &Value<'_>,
    buffer: &mut Vec<u8>,
    index: usize,
) -> bool {
    let entry_start = buffer.len();
    if index > 0 {
        buffer.put(COMMA);
    }
    if !project_requires(possible_types, requires, entity, buffer, true, None) {
        buffer.truncate(entry_start);
        return false;
    }
    true
}

/// A key over the values a flat `requires` program reads, cheap enough to recognise a repeat
/// entity before paying to project it.
///
/// `None` unless every step is a leaf over a scalar slot, or a type condition over more of
/// the same. A nested `Enter` or a list at a leaf slot makes the written bytes depend on more
/// than one value per slot, and those keep projecting first and deduplicating on the bytes.
///
/// `__typename` is hashed whether or not the program selects it, because `project_requires`
/// writes it from slot 0 whenever it is there.
///
/// The key covers each value's variant as well as its payload, which makes it *more*
/// discriminating than the bytes, never less. Two entities with the same key therefore
/// project to the same bytes; two that would project alike from differently-typed values
/// simply fail to collapse, which costs one extra entry in `representations` and nothing
/// else. That asymmetry is the point -- the dangerous direction is impossible.
fn flat_requires_key(steps: &[RequiresStep], entity: &Value<'_>) -> Option<u64> {
    let slots = entity.as_object()?;
    let mut key = scalar_key(slots.get(TYPENAME_SLOT).unwrap_or(&Value::Absent))?;
    hash_flat_steps(steps, slots, &mut key)?;
    Some(key)
}

/// Mixes in the slots a flat program reads, or gives up.
fn hash_flat_steps(steps: &[RequiresStep], slots: &[Value<'_>], key: &mut u64) -> Option<()> {
    for step in steps {
        match step {
            RequiresStep::Leaf { slot, .. } => {
                *key = mix(
                    *key,
                    scalar_key(slots.get(*slot).unwrap_or(&Value::Absent))?,
                );
            }
            // A type condition reads the same object, and which branch runs is decided by a
            // `__typename` this key already covers against a condition fixed in the program.
            // Slots under a branch that does not run are mixed in anyway, which can only cost
            // a missed collapse.
            RequiresStep::OnType {
                typename_slot,
                steps,
                ..
            } => {
                if let Some(slot) = typename_slot {
                    *key = mix(
                        *key,
                        scalar_key(slots.get(*slot).unwrap_or(&Value::Absent))?,
                    );
                }
                hash_flat_steps(steps, slots, key)?;
            }
            // A nested object or a list under a leaf would need the same reasoning one level
            // down; those keep projecting first and deduplicating on the bytes.
            RequiresStep::Enter { .. } => return None,
        }
    }
    Some(())
}

/// Identifies one scalar slot by variant and payload, or gives up on a container.
///
/// Every variant gets its own tag, so two values that write differently can never share a key.
#[inline]
fn scalar_key(value: &Value<'_>) -> Option<u64> {
    let (tag, payload) = match value {
        Value::Absent => (0, 0),
        Value::Null => (1, 0),
        Value::Bool(b) => (2, *b as u64),
        Value::I64(n) => (3, *n as u64),
        Value::U64(n) => (4, *n),
        Value::F64(n) => (5, n.to_bits()),
        Value::String(s) => (6, xxh3_64(s.as_bytes())),
        Value::RawJson(raw) => (7, xxh3_64(raw.as_bytes())),
        Value::Array(_) | Value::Object(_) => return None,
    };
    Some(mix(tag, payload))
}

/// One round of avalanche over a running accumulator.
///
/// Written out rather than reached for through `Hasher`: this runs once per required field
/// per entity, and building an `AHasher` per entity cost more than the projection it was
/// meant to skip -- 1000 unique entities went from 62 to 114 us.
#[inline]
fn mix(state: u64, value: u64) -> u64 {
    let mut hash = state.rotate_left(27) ^ value;
    hash = hash.wrapping_mul(0x9e37_79b1_85eb_ca87);
    hash ^= hash >> 31;
    hash
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
        Value::RawJson(raw) => {
            write_response_key(first, response_key, buffer);
            buffer.put_slice(raw.as_bytes());
        }
        Value::Array(items) => {
            write_response_key(first, response_key, buffer);
            buffer.put(OPEN_BRACKET);

            let mut first = true;
            for item in items.iter() {
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
    use super::{project_requires, push_representation};
    use crate::introspection::schema::PossibleTypes;
    use ahash::HashMap as AHashMap;
    use graphql_tools::parser::query;
    use hive_router_query_planner::ast::selection_set::SelectionSet;
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

    /// The value-key path must produce exactly what projecting every entity and
    /// deduplicating on the resulting bytes produces: the same `representations` array, and
    /// the same grouping of entities onto entries.
    ///
    /// The reference is written out independently rather than by disabling the fast path, so
    /// it checks the fast path against the meaning of the operation and not against another
    /// copy of itself. Returns the grouping so each caller can also pin what it expects.
    fn assert_dedup_matches_reference(
        requires: &str,
        entities: &[sonic_rs::Value],
    ) -> (Vec<Option<usize>>, usize) {
        let selections = requires_from_str(requires);
        let shape = response_shape_for_selections(&selections);
        let compiled = compile_requires(&selections, &shape);
        let possible_types = PossibleTypes::default();

        let parsed: Vec<_> = entities
            .iter()
            .map(|entity| {
                SubgraphResponse::parse_data_with_shape(
                    &sonic_rs::to_string(entity).unwrap(),
                    &shape,
                )
            })
            .collect();

        let arena = bumpalo::Bump::new();
        let mut buffer = Vec::new();
        let mut seen: AHashMap<u64, usize> = AHashMap::default();
        let mut next_index = 0usize;
        let keys: Vec<_> = parsed
            .iter()
            .map(|owned| {
                push_representation(
                    &possible_types,
                    &compiled,
                    &[],
                    &arena,
                    &owned.data,
                    &mut buffer,
                    &mut seen,
                    &mut next_index,
                )
            })
            .collect();

        // Reference: project each entity on its own, then deduplicate on the exact bytes.
        let mut entries: Vec<Vec<u8>> = Vec::new();
        let mut reference_groups: Vec<Option<usize>> = Vec::new();
        for owned in &parsed {
            let mut one = Vec::new();
            if !project_requires(
                &possible_types,
                &compiled,
                &owned.data,
                &mut one,
                true,
                None,
            ) {
                reference_groups.push(None);
                continue;
            }
            let group = match entries.iter().position(|entry| *entry == one) {
                Some(group) => group,
                None => {
                    entries.push(one);
                    entries.len() - 1
                }
            };
            reference_groups.push(Some(group));
        }

        // The returned keys are opaque identifiers, so compare how they group entities
        // rather than their values.
        let mut order: Vec<u64> = Vec::new();
        let groups: Vec<Option<usize>> = keys
            .iter()
            .map(|key| {
                key.map(|key| match order.iter().position(|seen| *seen == key) {
                    Some(group) => group,
                    None => {
                        order.push(key);
                        order.len() - 1
                    }
                })
            })
            .collect();

        assert_eq!(
            String::from_utf8(buffer).unwrap(),
            String::from_utf8(entries.join(&b','.to_owned())).unwrap(),
            "representations differ for `{requires}`"
        );
        assert_eq!(
            groups, reference_groups,
            "grouping differs for `{requires}`"
        );
        assert_eq!(next_index, entries.len());

        (groups, entries.len())
    }

    fn dedup_test_entities() -> Vec<sonic_rs::Value> {
        vec![
            json!({ "__typename": "Product", "upc": "1" }),
            json!({ "__typename": "Product", "upc": "2" }),
            // Same key field, different type: must not collapse into the entry above.
            json!({ "__typename": "Other", "upc": "1" }),
            // An answered `null` is kept in the representation; a field no response filled in
            // is left out, so these two must stay apart -- and the second projects to nothing.
            json!({ "__typename": "Product", "upc": null }),
            json!({ "__typename": "Product" }),
            // Repeats, now that the map has other entries in it.
            json!({ "__typename": "Product", "upc": "1" }),
            json!({ "__typename": "Other", "upc": "1" }),
            // A number where the others had a string: same digits, different bytes.
            json!({ "__typename": "Product", "upc": 1 }),
        ]
    }

    #[test]
    fn value_key_dedup_matches_projecting_every_entity() {
        let entities = dedup_test_entities();
        let (groups, distinct) = assert_dedup_matches_reference("__typename upc", &entities);

        // Guard the test itself: it is only meaningful if these entities really do exercise
        // both collapsing and staying apart.
        assert_eq!(distinct, 5);
        assert_eq!(groups[5], groups[0]);
        assert_eq!(groups[6], groups[2]);
        assert_eq!(groups[4], None);
        assert_ne!(groups[0], groups[7]);
    }

    /// A query plan writes `requires` as an inline fragment on the entity type, so the type
    /// condition — not the bare field list above — is the shape that actually reaches the
    /// router.
    #[test]
    fn value_key_dedup_handles_a_type_condition() {
        let entities = dedup_test_entities();
        let (groups, distinct) =
            assert_dedup_matches_reference("... on Product { __typename upc }", &entities);

        // Same grouping as the bare field list: the condition is decided by a `__typename`
        // the key already covers, so it changes what is written, never how entities collapse.
        assert_eq!(distinct, 4, "groups: {groups:?}");
        assert_eq!(groups[5], groups[0]);
        assert_eq!(groups[6], groups[2]);
        assert_eq!(groups[4], None);
        assert_ne!(groups[0], groups[7]);
    }
}
