use query_planner::ast::selection_item::SelectionItem;
use std::io::Write;

use crate::{
    consts::TYPENAME_FIELD_NAME, json_writer::write_and_escape_string, response::value::Value,
    schema::metadata::PossibleTypes,
};

pub struct RequestProjectionContext<'a> {
    possible_types: &'a PossibleTypes<'a>,
}

pub fn project_requires(
    ctx: &RequestProjectionContext,
    requires_selections: &Vec<SelectionItem>,
    entity: &Value,
    buffer: &mut Vec<u8>,
    first: bool,
    response_key: Option<&str>,
) -> bool {
    project_requires_internal(
        ctx,
        requires_selections,
        entity,
        buffer,
        first,
        response_key,
    )
}

fn project_requires_internal(
    ctx: &RequestProjectionContext,
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
            if !first {
                buffer.push(b',');
            }
            if let Some(response_key) = response_key {
                buffer.push(b'"');
                buffer.write(response_key.as_bytes()).unwrap();
                buffer.push(b'"');
                buffer.push(b':');
                buffer
                    .write(if b == &&true { b"true" } else { b"false" })
                    .unwrap();
            } else {
                buffer
                    .write(if b == &&true { b"true" } else { b"false" })
                    .unwrap();
            }
        }
        Value::F64(n) => {
            if !first {
                buffer.push(b',');
            }
            if let Some(response_key) = response_key {
                buffer.push(b'"');
                buffer.write(response_key.as_bytes()).unwrap();
                buffer.write(b"\":").unwrap();
            }

            write!(buffer, "{}", n).unwrap()
        }
        Value::I64(n) => {
            if !first {
                buffer.push(b',');
            }
            if let Some(response_key) = response_key {
                buffer.push(b'"');
                buffer.write(response_key.as_bytes()).unwrap();
                buffer.write(b"\":").unwrap();
            }

            write!(buffer, "{}", n).unwrap()
        }
        Value::U64(n) => {
            if !first {
                buffer.push(b',');
            }
            if let Some(response_key) = response_key {
                buffer.push(b'"');
                buffer.write(response_key.as_bytes()).unwrap();
                buffer.write(b"\":").unwrap();
            }

            write!(buffer, "{}", n).unwrap()
        }
        Value::String(s) => {
            if !first {
                buffer.push(b',');
            }
            if let Some(response_key) = response_key {
                buffer.push(b'"');
                buffer.write(response_key.as_bytes()).unwrap();
                buffer.write(b"\":").unwrap();
            }
            write_and_escape_string(buffer, s).unwrap();
        }
        Value::Array(entity_array) => {
            if !first {
                buffer.push(b',');
            }
            if let Some(response_key) = response_key {
                buffer.push(b'"');
                buffer.write(response_key.as_bytes()).unwrap();
                buffer.write(b"\":[").unwrap();
            } else {
                buffer.push(b'[');
            }
            let mut first = true;
            for entity_item in entity_array {
                let projected = project_requires_internal(
                    ctx,
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
            buffer.push(b']');
        }
        Value::Object(entity_obj) => {
            if requires_selections.is_empty() {
                // It is probably a scalar with an object value, so we write it directly
                // buffer.push_str(&serde_json::to_string(entity_obj).unwrap());
                buffer.write(b"{}").unwrap();
                return true;
            }
            if entity_obj.is_empty() {
                return false;
            }

            let parent_first = first;
            let mut first = true;
            project_requires_map_mut(
                ctx,
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
                buffer.push(b'}');
            }
        }
    };
    true
}

fn project_requires_map_mut(
    ctx: &RequestProjectionContext,
    requires_selections: &Vec<SelectionItem>,
    entity_obj: &Vec<(&str, Value<'_>)>,
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

                let original = entity_obj
                    .iter()
                    .find(|(k, _)| k == field_name)
                    .map(|(_, val)| val)
                    .unwrap_or(
                        entity_obj
                            .iter()
                            .find(|(k, _)| k == &response_key)
                            .map(|(_, val)| val)
                            .unwrap_or(&Value::Null),
                    );

                if original.is_null() {
                    continue;
                }

                if *first {
                    if !parent_first {
                        buffer.push(b',');
                    }
                    if let Some(parent_response_key) = parent_response_key {
                        buffer.push(b'"');
                        buffer.write(parent_response_key.as_bytes()).unwrap();
                        buffer.write(b"\":").unwrap();
                    }
                    buffer.push(b'{');
                    // Write __typename only if the object has other fields
                    if let Some(type_name) = entity_obj
                        .iter()
                        .find(|(key, _)| key == &TYPENAME_FIELD_NAME)
                        .and_then(|(_, val)| val.as_str())
                    {
                        buffer.write(b"\"__typename\":").unwrap();
                        write_and_escape_string(buffer, type_name).unwrap();
                        buffer.push(b',');
                    }
                }

                // To avoid writing empty fields, we write to a temporary buffer first
                project_requires_internal(
                    ctx,
                    &requires_selection.selections.items,
                    original,
                    buffer,
                    *first,
                    Some(response_key),
                );
                *first = false;
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
                if ctx
                    .possible_types
                    .entity_satisfies_type_condition(type_name, type_condition)
                    || ctx
                        .possible_types
                        .entity_satisfies_type_condition(type_condition, type_name)
                {
                    project_requires_map_mut(
                        ctx,
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
                // We only minify the queries to subgraphs, so we never have fragment spreads here
                unreachable!("Fragment spreads should not exist in FetchNode::requires.");
            }
        }
    }
}
