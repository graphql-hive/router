use bytes::BufMut;
use hive_router_query_planner::ast::selection_item::SelectionItem;

use crate::{
    introspection::schema::PossibleTypes,
    json_writer::{write_and_escape_string, write_f64, write_i64, write_u64},
    projection::response::serialize_value_to_buffer,
    response::value::{Value, ValueObject},
    utils::consts::{
        CLOSE_BRACE, CLOSE_BRACKET, COLON, COMMA, FALSE, NULL, OPEN_BRACE, OPEN_BRACKET, QUOTE,
        TRUE, TYPENAME, TYPENAME_FIELD_NAME,
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

pub fn project_requires(
    possible_types: &PossibleTypes,
    requires_selections: &Vec<SelectionItem>,
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
            project_requires_map_mut(
                possible_types,
                requires_selections,
                entity_obj,
                buffer,
                &mut first,
                response_key,
                parent_first,
            );
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

fn project_requires_map_mut(
    possible_types: &PossibleTypes,
    requires_selections: &Vec<SelectionItem>,
    entity_obj: &ValueObject<'_>,
    buffer: &mut Vec<u8>,
    first: &mut bool,
    parent_response_key: Option<&str>,
    parent_first: bool,
) {
    for requires_selection in requires_selections {
        match &requires_selection {
            SelectionItem::Field(requires_selection) => {
                let field_name = &requires_selection.name;
                let response_key = requires_selection.selection_identifier();
                if response_key == TYPENAME_FIELD_NAME {
                    // Skip __typename field, it is handled separately
                    continue;
                }

                let original = entity_obj.get(field_name).or_else(|| {
                    if response_key == TYPENAME_FIELD_NAME {
                        None
                    } else {
                        entity_obj.get(response_key)
                    }
                });

                let Some(original) = original else {
                    continue;
                };

                // In most requests, required fields are present and projection succeeds.
                // If projection ends up writing nothing, we rewind to this offset.
                let mut object_start_offset = None;

                if *first {
                    object_start_offset = Some(buffer.len());
                    write_response_key(parent_first, parent_response_key, buffer);
                    buffer.put(OPEN_BRACE);
                    // Write __typename only if the object has other fields
                    if let Some(type_name) = entity_obj.type_name() {
                        buffer.put(QUOTE);
                        buffer.put(TYPENAME);
                        buffer.put(QUOTE);
                        buffer.put(COLON);
                        write_and_escape_string(buffer, type_name);
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
                    &requires_selection.selections.items,
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
            SelectionItem::InlineFragment(requires_selection) => {
                let type_condition = &requires_selection.type_condition;

                let type_name = match entity_obj.type_name() {
                    Some(type_name) => type_name,
                    _ => type_condition,
                };
                // For projection, both sides of the condition are valid
                if possible_types.entity_satisfies_type_condition(type_name, type_condition)
                    || possible_types.entity_satisfies_type_condition(type_condition, type_name)
                {
                    project_requires_map_mut(
                        possible_types,
                        &requires_selection.selections.items,
                        entity_obj,
                        buffer,
                        first,
                        parent_response_key,
                        parent_first,
                    );
                }
            }
            SelectionItem::FragmentSpread(_name_ref) => {
                // We only minify the queries to subgraphs, so we never have fragment spreads here.
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::project_requires;
    use crate::{introspection::schema::PossibleTypes, response::value::Value};
    use graphql_tools::parser::query;
    use hive_router_query_planner::ast::{
        selection_item::SelectionItem, selection_set::SelectionSet,
    };
    use hive_router_query_planner::utils::parsing::parse_operation;
    use sonic_rs::json;

    fn requires_from_str(requires: &str) -> Vec<SelectionItem> {
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
        selection_set.items
    }

    fn project_requires_pretty(requires: &str, entity_json: sonic_rs::Value) -> Option<String> {
        let requires = requires_from_str(requires);
        let entity = Value::from(entity_json.as_ref());

        let mut buffer = Vec::new();
        let projected = project_requires(
            &PossibleTypes::default(),
            &requires,
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
            "contactOptions": null,
            "id": "1"
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
}
