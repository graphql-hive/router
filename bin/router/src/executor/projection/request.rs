use crate::query_planner::ast::requires::{RequiresSelection, RequiresSelectionSetRef};
use bytes::BufMut;

use crate::executor::{
    introspection::schema::PossibleTypes,
    json_writer::{write_and_escape_string, write_f64, write_i64, write_u64},
    projection::response::serialize_value_to_buffer,
    response::value::Value,
    utils::consts::{
        CLOSE_BRACE, CLOSE_BRACKET, COLON, COMMA, FALSE, NULL, OPEN_BRACE, OPEN_BRACKET, QUOTE,
        TRUE, TYPENAME_FIELD_NAME, TYPENAME_JSON_FIELD,
    },
};

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

pub fn project_requires(
    possible_types: &PossibleTypes,
    requires_selections: RequiresSelectionSetRef<'_>,
    entity: &Value,
    buffer: &mut Vec<u8>,
    first: bool,
    response_key: Option<&str>,
) -> bool {
    match entity {
        Value::Null => {
            return false;
        }
        Value::Bool(b) => {
            write_response_key(first, response_key, buffer);
            buffer.put(if b == &true { TRUE } else { FALSE });
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
        Value::Array(entity_array) => {
            write_response_key(first, response_key, buffer);
            buffer.put(OPEN_BRACKET);

            let mut first = true;
            for entity_item in entity_array {
                let projected = project_requires(
                    possible_types,
                    requires_selections,
                    entity_item,
                    buffer,
                    first,
                    None,
                );
                if projected {
                    // Only update `first` if we actually write something
                    first = false;
                }
            }
            buffer.put(CLOSE_BRACKET);
        }
        Value::Object(entity_obj) => {
            if requires_selections.is_empty() {
                // It is probably a scalar with an object value, so we write it directly
                write_response_key(first, response_key, buffer);
                serialize_value_to_buffer(entity, buffer);
                return true;
            }
            if entity_obj.is_empty() {
                return false;
            }

            let parent_first = first;
            let mut first = true;
            let applied = project_requires_map_mut(
                possible_types,
                requires_selections,
                entity_obj,
                buffer,
                &mut first,
                response_key,
                parent_first,
            );
            // `__typename` is written along with the object's first field. When it's the only
            // selected field that applies.
            // An object that misses a selected field, like an entity without its key, is left out.
            if first && applied.typename && !applied.missing_field {
                if let Some(type_name) =
                    Value::object_get(entity_obj, TYPENAME_FIELD_NAME).and_then(Value::as_str)
                {
                    write_response_key(parent_first, response_key, buffer);
                    buffer.put(OPEN_BRACE);
                    write_typename_field(buffer, type_name);
                    first = false;
                }
            }

            if first {
                // If no fields were projected, "first" is still true,
                // so we skip writing the closing brace
                return false;
            } else {
                buffer.put(CLOSE_BRACE);
            }
        }
    };
    true
}

/// What the selections that apply to an object ask for.
#[derive(Default)]
struct AppliedSelections {
    /// `__typename` is one of them.
    typename: bool,
    /// One of the fields isn't in the object.
    missing_field: bool,
}

fn project_requires_map_mut(
    possible_types: &PossibleTypes,
    requires_selections: RequiresSelectionSetRef<'_>,
    entity_obj: &Vec<(&str, Value<'_>)>,
    buffer: &mut Vec<u8>,
    first: &mut bool,
    parent_response_key: Option<&str>,
    parent_first: bool,
) -> AppliedSelections {
    // First, check if __typename is present in the entity object, we'll use it later
    let type_name = Value::object_get(entity_obj, TYPENAME_FIELD_NAME).and_then(Value::as_str);
    let mut applied = AppliedSelections::default();

    for requires_selection in requires_selections.iter() {
        match requires_selection {
            RequiresSelection::Field {
                name: field_name,
                alias,
                selections,
                ..
            } => {
                let response_key = alias.unwrap_or(field_name);
                if response_key == TYPENAME_FIELD_NAME {
                    applied.typename = true;
                    continue;
                }

                let original = Value::object_get(entity_obj, field_name)
                    .or_else(|| Value::object_get(entity_obj, response_key));

                let Some(original) = original else {
                    applied.missing_field = true;
                    continue;
                };

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
                    write_response_key(*first, Some(response_key), buffer);
                    buffer.put(NULL);
                    *first = false;
                    continue;
                }

                let projected = project_requires(
                    possible_types,
                    selections,
                    original,
                    buffer,
                    *first,
                    Some(response_key),
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
            RequiresSelection::InlineFragment {
                type_condition,
                selections,
                ..
            } => {
                let type_name = type_name.unwrap_or(type_condition);
                // For projection, both sides of the condition are valid
                if possible_types.entity_satisfies_type_condition(type_name, type_condition)
                    || possible_types.entity_satisfies_type_condition(type_condition, type_name)
                {
                    let fragment = project_requires_map_mut(
                        possible_types,
                        selections,
                        entity_obj,
                        buffer,
                        first,
                        parent_response_key,
                        parent_first,
                    );
                    applied.typename |= fragment.typename;
                    applied.missing_field |= fragment.missing_field;
                }
            }
            RequiresSelection::FragmentSpread(_) => {
                // We only minify the queries to subgraphs, so we never have fragment spreads here.
            }
        }
    }
    applied
}

#[cfg(test)]
mod tests {
    use super::project_requires;
    use crate::executor::{introspection::schema::PossibleTypes, response::value::Value};
    use crate::query_planner::ast::{requires::RequiresSelectionSet, selection_set::SelectionSet};
    use crate::query_planner::utils::parsing::parse_operation;
    use graphql_tools::parser::query;
    use sonic_rs::json;

    fn requires_from_str(requires: &str) -> RequiresSelectionSet {
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

        let selection_set: SelectionSet = selection_set.into();
        RequiresSelectionSet::from(&selection_set)
    }

    fn project_requires_pretty(requires: &str, entity_json: sonic_rs::Value) -> Option<String> {
        let requires = requires_from_str(requires);
        let entity = Value::from(entity_json.as_ref());

        let mut buffer = Vec::new();
        let projected = project_requires(
            &PossibleTypes::default(),
            requires.root_selections(),
            &entity,
            &mut buffer,
            true,
            None,
        );

        if !projected {
            return None;
        }

        let json: Value = sonic_rs::from_slice(&buffer).unwrap();
        Some(sonic_rs::to_string_pretty(&json).unwrap())
    }

    #[test]
    fn project_requires_preserves_aliases_at_each_depth() {
        let projected = project_requires_pretty(
            "key: id nested { renamed: value } ... on Product { code: upc }",
            json!({
                "__typename": "Product",
                "id": "original",
                "key": "fallback",
                "nested": { "renamed": 2 },
                "upc": "123"
            }),
        )
        .expect("projection should produce output");

        let actual: serde_json::Value = serde_json::from_str(&projected).unwrap();
        assert_eq!(
            actual,
            serde_json::json!({
                "__typename": "Product",
                "key": "original",
                "nested": { "renamed": 2 },
                "code": "123"
            })
        );
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

    /// An object of none of its fragments' types still has `__typename`, which is all the
    /// selection asks of it, so it's kept, in a field or in a list. An object that misses a
    /// selected field is still left out, so an entity without its key isn't sent.
    #[test]
    fn project_requires_object_of_none_of_the_fragment_types() {
        let requires = "... on Listing { __typename pet { __typename ... on Dog { tricks } ... on Cat { whiskers } } id }";

        // A Bird is neither a Dog nor a Cat
        insta::assert_snapshot!(
          &project_requires_pretty(
              requires,
              json!({
                  "__typename": "Listing",
                  "id": "l3",
                  "pet": { "__typename": "Bird", "id": "b1" }
              }),
          )
          .expect("projection should produce output"),
          @r#"
          {
            "__typename": "Listing",
            "pet": {
              "__typename": "Bird"
            },
            "id": "l3"
          }
        "#);

        // In a list, the Bird keeps its place
        insta::assert_snapshot!(
          &project_requires_pretty(
              "... on Shelter { __typename pets { __typename ... on Dog { tricks } ... on Cat { whiskers } } id }",
              json!({
                  "__typename": "Shelter",
                  "id": "s1",
                  "pets": [
                      { "__typename": "Cat", "id": "c1", "whiskers": 12 },
                      { "__typename": "Bird", "id": "b1" },
                      { "__typename": "Dog", "id": "d1", "tricks": 3 }
                  ]
              }),
          )
          .expect("projection should produce output"),
          @r#"
          {
            "__typename": "Shelter",
            "pets": [
              {
                "__typename": "Cat",
                "whiskers": 12
              },
              {
                "__typename": "Bird"
              },
              {
                "__typename": "Dog",
                "tricks": 3
              }
            ],
            "id": "s1"
          }
        "#);

        // A Cat without its whiskers is left out
        insta::assert_snapshot!(
          &project_requires_pretty(
              requires,
              json!({
                  "__typename": "Listing",
                  "id": "l1",
                  "pet": { "__typename": "Cat", "id": "c1" }
              }),
          )
          .expect("projection should produce output"),
          @r#"
          {
            "__typename": "Listing",
            "id": "l1"
          }
        "#);

        // A Book without its key isn't sent
        let pretty = project_requires_pretty(
            "... on Book { __typename id }",
            json!({ "__typename": "Book", "upc": "b3" }),
        );
        assert_eq!(pretty, None);
    }
}
