use std::collections::HashMap;
use std::io::Write;

use tracing::{instrument, warn};

use crate::consts::TYPENAME_FIELD_NAME;
use crate::json_writer::write_and_escape_string;
use crate::projection::plan::{FieldProjectionConditionError, FieldProjectionPlan};
use crate::response::value::Value;

#[instrument(level = "trace", skip_all)]
pub fn project_by_operation(
    data: &Value,
    // errors: &mut Vec<GraphQLError>,
    // extensions: &HashMap<String, serde_json::Value>,
    operation_type_name: &str,
    selections: &Vec<FieldProjectionPlan>,
    variable_values: &Option<HashMap<String, serde_json::Value>>,
) -> Vec<u8> {
    // We may want to remove it, but let's see.
    let mut buffer: Vec<u8> = Vec::with_capacity(4096);

    buffer.write_all(b"{\"data\":").unwrap();

    if let Some(data_map) = data.as_object() {
        let mut first = true;
        project_selection_set_with_map(
            data_map,
            // errors,
            selections,
            variable_values,
            operation_type_name,
            &mut buffer,
            &mut first, // Start with first as true to add the opening brace
        );
        if !first {
            buffer.push(b'}');
        } else {
            // If no selections were made, we should return an empty object
            buffer.write_all(b"{}").unwrap();
        }
    }

    // if !errors.is_empty() {
    //     write!(
    //         buffer,
    //         ",\"errors\":{}",
    //         serde_json::to_string(&errors).unwrap()
    //     )
    //     .unwrap();
    // }
    // if !extensions.is_empty() {
    //     write!(
    //         buffer,
    //         ",\"extensions\":{}",
    //         serde_json::to_string(&extensions).unwrap()
    //     )
    //     .unwrap();
    // }

    buffer.push(b'}');
    buffer
}

fn project_selection_set(
    data: &Value,
    // errors: &mut Vec<GraphQLError>,
    selection: &FieldProjectionPlan,
    variable_values: &Option<HashMap<String, serde_json::Value>>,
    buffer: &mut Vec<u8>,
) -> () {
    match data {
        Value::Null => buffer.write(b"null").unwrap(),
        Value::Bool(true) => buffer.write(b"true").unwrap(),
        Value::Bool(false) => buffer.write(b"false").unwrap(),
        Value::U64(num) => buffer.write(num.to_string().as_bytes()).unwrap(),
        Value::I64(num) => buffer.write(num.to_string().as_bytes()).unwrap(),
        Value::F64(num) => buffer.write(num.to_string().as_bytes()).unwrap(),
        Value::String(value) => write_and_escape_string(buffer, value).unwrap(),
        Value::Array(arr) => {
            buffer.push(b'[');
            let mut first = true;
            for item in arr.iter() {
                if !first {
                    buffer.push(b',');
                }
                project_selection_set(item, selection, variable_values, buffer);
                first = false;
            }
            buffer.push(b']');
            0
        }
        Value::Object(obj) => {
            match selection.selections.as_ref() {
                Some(selections) => {
                    let mut first = true;
                    let type_name = obj
                        .iter()
                        .find(|(k, _)| k == &TYPENAME_FIELD_NAME)
                        .and_then(|(_, val)| val.as_str())
                        .unwrap_or(selection.field_type);
                    project_selection_set_with_map(
                        obj,
                        selections,
                        variable_values,
                        type_name,
                        buffer,
                        &mut first,
                    );
                    if !first {
                        buffer.push(b'}');
                    } else {
                        // If no selections were made, we should return an empty object
                        buffer.write(b"{}").unwrap();
                    }
                    0
                }
                None => {
                    // If the selection set is not projected, we should return null
                    buffer.write(b"null").unwrap();
                    0
                }
            }
        }
    };
}

// TODO: simplfy args
#[allow(clippy::too_many_arguments)]
fn project_selection_set_with_map(
    obj: &Vec<(&str, Value)>,
    // errors: &mut Vec<GraphQLError>,
    selections: &Vec<FieldProjectionPlan>,
    variable_values: &Option<HashMap<String, serde_json::Value>>,
    parent_type_name: &str,
    buffer: &mut Vec<u8>,
    first: &mut bool,
) {
    for selection in selections {
        let field_val = obj
            .iter()
            .find_map(|(key, val)| {
                if key == &selection.field_name {
                    Some(val)
                } else {
                    None
                }
            })
            .or_else(|| {
                obj.iter().find_map(|(key, val)| {
                    if key == &selection.response_key {
                        Some(val)
                    } else {
                        None
                    }
                })
            });
        match selection.conditions.check(
            parent_type_name,
            &selection.field_type,
            &field_val,
            variable_values,
        ) {
            Ok(_) => {
                if *first {
                    buffer.push(b'{');
                } else {
                    buffer.push(b',');
                }
                *first = false;

                buffer.push(b'"');
                buffer.write(selection.response_key.as_bytes()).unwrap();
                buffer.write(b"\":").unwrap();

                if let Some(field_val) = field_val {
                    project_selection_set(field_val, selection, variable_values, buffer);
                } else if selection.field_name == TYPENAME_FIELD_NAME {
                    // If the field is TYPENAME_FIELD, we should set it to the parent type name
                    buffer.push(b'"');
                    buffer.write(parent_type_name.as_bytes()).unwrap();
                    buffer.push(b'"');
                } else {
                    // If the field is not found in the object, set it to Null
                    buffer.write(b"null").unwrap();
                }
            }
            Err(FieldProjectionConditionError::Skip) => {
                // Skip this field
                continue;
            }
            Err(FieldProjectionConditionError::InvalidParentType) => {
                // Skip this field as the parent type does not match
                continue;
            }
            Err(FieldProjectionConditionError::InvalidEnumValue) => {
                if *first {
                    buffer.push(b'{');
                } else {
                    buffer.push(b',');
                }
                *first = false;

                buffer.push(b'"');
                buffer.write(selection.response_key.as_bytes()).unwrap();
                buffer.write(b"\":null").unwrap();
                // errors.push(GraphQLError {
                //     message: "Value is not a valid enum value".to_string(),
                //     locations: None,
                //     path: None,
                //     extensions: None,
                // });
            }
            Err(FieldProjectionConditionError::InvalidFieldType) => {
                if *first {
                    buffer.push(b'{');
                } else {
                    buffer.push(b',');
                }
                *first = false;

                // Skip this field as the field type does not match
                buffer.push(b'"');
                buffer.write(selection.response_key.as_bytes()).unwrap();
                buffer.write(b"\":null").unwrap();
            }
        }
    }
}
