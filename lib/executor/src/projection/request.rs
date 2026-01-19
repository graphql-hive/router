use bytes::BufMut;
use hive_router_query_planner::ast::selection_item::SelectionItem;

use crate::{
    introspection::schema::PossibleTypes,
    json_writer::{write_and_escape_string, write_f64, write_i64, write_u64},
    projection::{error::ProjectionError, response::serialize_value_to_buffer},
    response::value::Value,
    utils::consts::{
        CLOSE_BRACE, CLOSE_BRACKET, COLON, COMMA, FALSE, OPEN_BRACE, OPEN_BRACKET, QUOTE, TRUE,
        TYPENAME, TYPENAME_FIELD_NAME,
    },
};

pub fn project_requires(
    possible_types: &PossibleTypes,
    requires_selections: &Vec<SelectionItem>,
    entity: &Value,
    buffer: &mut Vec<u8>,
    first: bool,
    response_key: Option<&str>,
) -> Result<bool, ProjectionError> {
    project_requires_internal(
        possible_types,
        requires_selections,
        entity,
        buffer,
        first,
        response_key,
    )
}

fn project_requires_internal(
    possible_types: &PossibleTypes,
    requires_selections: &Vec<SelectionItem>,
    entity: &Value,
    buffer: &mut Vec<u8>,
    first: bool,
    response_key: Option<&str>,
) -> Result<bool, ProjectionError> {
    match entity {
        Value::Null => {
            return Ok(false);
        }
        Value::Bool(b) => {
            if !first {
                buffer.put(COMMA);
            }
            if let Some(response_key) = response_key {
                buffer.put(QUOTE);
                buffer.put(response_key.as_bytes());
                buffer.put(QUOTE);
                buffer.put(COLON);
                buffer.put(if b == &true { TRUE } else { FALSE });
            } else {
                buffer.put(if b == &true { TRUE } else { FALSE });
            }
        }
        Value::F64(n) => {
            if !first {
                buffer.put(COMMA);
            }
            if let Some(response_key) = response_key {
                buffer.put(QUOTE);
                buffer.put(response_key.as_bytes());
                buffer.put(QUOTE);
                buffer.put(COLON);
            }
            write_f64(buffer, *n);
        }
        Value::I64(n) => {
            if !first {
                buffer.put(COMMA);
            }
            if let Some(response_key) = response_key {
                buffer.put(QUOTE);
                buffer.put(response_key.as_bytes());
                buffer.put(QUOTE);
                buffer.put(COLON);
            }
            write_i64(buffer, *n);
        }
        Value::U64(n) => {
            if !first {
                buffer.put(COMMA);
            }
            if let Some(response_key) = response_key {
                buffer.put(QUOTE);
                buffer.put(response_key.as_bytes());
                buffer.put(QUOTE);
                buffer.put(COLON);
            }
            write_u64(buffer, *n);
        }
        Value::String(s) => {
            if !first {
                buffer.put(COMMA);
            }
            if let Some(response_key) = response_key {
                buffer.put(QUOTE);
                buffer.put(response_key.as_bytes());
                buffer.put(QUOTE);
                buffer.put(COLON);
            }
            write_and_escape_string(buffer, s);
        }
        Value::Array(entity_array) => {
            if !first {
                buffer.put(COMMA);
            }
            if let Some(response_key) = response_key {
                buffer.put(QUOTE);
                buffer.put(response_key.as_bytes());
                buffer.put(QUOTE);
                buffer.put(COLON);
            }
            buffer.put(OPEN_BRACKET);

            let mut first = true;
            for entity_item in entity_array {
                let projected = project_requires_internal(
                    possible_types,
                    requires_selections,
                    entity_item,
                    buffer,
                    first,
                    None,
                )?;
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
                serialize_value_to_buffer(entity, buffer);
                return Ok(true);
            }
            if entity_obj.is_empty() {
                return Ok(false);
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
            )?;
            if first {
                // If no fields were projected, "first" is still true,
                // so we skip writing the closing brace
                return Ok(false);
            } else {
                buffer.put(CLOSE_BRACE);
            }
        }
    };
    Ok(true)
}

fn project_requires_map_mut(
    possible_types: &PossibleTypes,
    requires_selections: &Vec<SelectionItem>,
    entity_obj: &Vec<(&str, Value<'_>)>,
    buffer: &mut Vec<u8>,
    first: &mut bool,
    parent_response_key: Option<&str>,
    parent_first: bool,
) -> Result<(), ProjectionError> {
    for requires_selection in requires_selections {
        match &requires_selection {
            SelectionItem::Field(requires_selection) => {
                let field_name = &requires_selection.name;
                let response_key = requires_selection.selection_identifier();
                if response_key == TYPENAME_FIELD_NAME {
                    // Skip __typename field, it is handled separately
                    continue;
                }

                let original = entity_obj
                    .binary_search_by_key(&field_name.as_str(), |(k, _)| k)
                    .ok()
                    .map(|idx| &entity_obj[idx].1)
                    .or_else(|| {
                        if response_key == TYPENAME_FIELD_NAME {
                            None
                        } else {
                            entity_obj
                                .binary_search_by_key(&response_key, |(k, _)| k)
                                .ok()
                                .map(|idx| &entity_obj[idx].1)
                        }
                    })
                    .unwrap_or(&Value::Null);

                if original.is_null() {
                    continue;
                }

                if *first {
                    if !parent_first {
                        buffer.put(COMMA);
                    }
                    if let Some(parent_response_key) = parent_response_key {
                        buffer.put(QUOTE);
                        buffer.put(parent_response_key.as_bytes());
                        buffer.put(QUOTE);
                        buffer.put(COLON);
                    }
                    buffer.put(OPEN_BRACE);
                    // Write __typename only if the object has other fields
                    if let Some(type_name) = entity_obj
                        .binary_search_by_key(&TYPENAME_FIELD_NAME, |(k, _)| k)
                        .ok()
                        .and_then(|idx| entity_obj[idx].1.as_str())
                    {
                        buffer.put(QUOTE);
                        buffer.put(TYPENAME);
                        buffer.put(QUOTE);
                        buffer.put(COLON);
                        write_and_escape_string(buffer, type_name);
                        // We wrote the first field
                        *first = false;
                    }
                }

                let projected = project_requires_internal(
                    possible_types,
                    &requires_selection.selections.items,
                    original,
                    buffer,
                    *first,
                    Some(response_key),
                )?;
                if projected {
                    *first = false;
                }
            }
            SelectionItem::InlineFragment(requires_selection) => {
                let type_condition = &requires_selection.type_condition;

                let type_name = match entity_obj
                    .iter()
                    .find(|(key, _)| key == &TYPENAME_FIELD_NAME)
                    .and_then(|(_, val)| val.as_str())
                {
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
                    )?;
                }
            }
            SelectionItem::FragmentSpread(_name_ref) => {
                // We only minify the queries to subgraphs, so we never have fragment spreads here.
            }
        }
    }
    Ok(())
}
