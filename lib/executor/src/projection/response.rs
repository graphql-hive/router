use crate::projection::error::ProjectionError;
use crate::projection::plan::{
    FieldProjectionCondition, FieldProjectionConditionError, FieldProjectionPlan, TypeCondition,
};
use crate::response::graphql_error::GraphQLError;
use crate::response::value::Value;
use bytes::{BufMut, Bytes, BytesMut};
use sonic_rs::JsonValueTrait;
use std::collections::HashMap;

use tracing::{instrument, warn};

use crate::json_writer::{write_and_escape_string, write_f64, write_i64, write_u64};
use crate::utils::consts::{
    CLOSE_BRACE, CLOSE_BRACKET, COLON, COMMA, EMPTY_OBJECT, FALSE, NULL, OPEN_BRACE, OPEN_BRACKET,
    QUOTE, TRUE, TYPENAME_FIELD_NAME,
};

#[instrument(level = "trace", skip_all)]
pub fn project_by_operation(
    data: &Value,
    errors: Vec<GraphQLError>,
    extensions: &Option<HashMap<String, sonic_rs::Value>>,
    operation_type_name: &str,
    selections: &Vec<FieldProjectionPlan>,
    variable_values: &Option<HashMap<String, sonic_rs::Value>>,
) -> Result<Bytes, ProjectionError> {
    let mut buffer = BytesMut::with_capacity(4096);
    buffer.put(OPEN_BRACE);
    buffer.put(QUOTE);
    buffer.put("data".as_bytes());
    buffer.put(QUOTE);
    buffer.put(COLON);

    let mut errors = errors;

    if let Some(data_map) = data.as_object() {
        // Start with first as true to add the opening brace
        let mut first = true;
        project_selection_set_with_map(
            data_map,
            &mut errors,
            selections,
            variable_values,
            operation_type_name,
            &mut buffer,
            &mut first,
        );
        if !first {
            buffer.put(CLOSE_BRACE);
        } else {
            // If no selections were made, we should return an empty object
            buffer.put(EMPTY_OBJECT);
        }
    }

    if !errors.is_empty() {
        buffer.put(COMMA);
        buffer.put(QUOTE);
        buffer.put("errors".as_bytes());
        buffer.put(QUOTE);
        buffer.put(COLON);
        buffer.put_slice(
            &sonic_rs::to_vec(&errors)
                .map_err(|e| ProjectionError::ErrorsSerializationFailure(e.to_string()))?,
        );
    }

    if let Some(ext) = extensions.as_ref() {
        if !ext.is_empty() {
            let serialized_extensions = sonic_rs::to_vec(ext)
                .map_err(|e| ProjectionError::ExtensionsSerializationFailure(e.to_string()))?;
            buffer.put(COMMA);
            buffer.put(QUOTE);
            buffer.put("extensions".as_bytes());
            buffer.put(QUOTE);
            buffer.put(COLON);
            buffer.put_slice(&serialized_extensions);
        }
    }

    buffer.put(CLOSE_BRACE);
    Ok(buffer.freeze())
}

fn project_selection_set(
    data: &Value,
    errors: &mut Vec<GraphQLError>,
    selection: &FieldProjectionPlan,
    variable_values: &Option<HashMap<String, sonic_rs::Value>>,
    buffer: &mut BytesMut,
) {
    match data {
        Value::Null => buffer.put(NULL),
        Value::Bool(true) => buffer.put(TRUE),
        Value::Bool(false) => buffer.put(FALSE),
        Value::U64(num) => write_u64(buffer, *num),
        Value::I64(num) => write_i64(buffer, *num),
        Value::F64(num) => write_f64(buffer, *num),
        Value::String(value) => write_and_escape_string(buffer, value),
        Value::Array(arr) => {
            buffer.put(OPEN_BRACKET);
            let mut first = true;
            for item in arr.iter() {
                if !first {
                    buffer.put(COMMA);
                }
                project_selection_set(item, errors, selection, variable_values, buffer);
                first = false;
            }
            buffer.put(CLOSE_BRACKET);
        }
        Value::Object(obj) => {
            match selection.selections.as_ref() {
                Some(selections) => {
                    let mut first = true;
                    let type_name = obj
                        .iter()
                        .position(|(k, _)| k == &TYPENAME_FIELD_NAME)
                        .and_then(|idx| obj[idx].1.as_str())
                        .unwrap_or(selection.field_type.as_str());
                    project_selection_set_with_map(
                        obj,
                        errors,
                        selections,
                        variable_values,
                        type_name,
                        buffer,
                        &mut first,
                    );
                    if !first {
                        buffer.put(CLOSE_BRACE);
                    } else {
                        // If no selections were made, we should return an empty object
                        buffer.put(EMPTY_OBJECT);
                    }
                }
                None => {
                    // If the selection set is not projected, we should return null
                    buffer.put(NULL)
                }
            }
        }
    };
}

// TODO: simplfy args
#[allow(clippy::too_many_arguments)]
fn project_selection_set_with_map(
    obj: &Vec<(&str, Value)>,
    errors: &mut Vec<GraphQLError>,
    selections: &Vec<FieldProjectionPlan>,
    variable_values: &Option<HashMap<String, sonic_rs::Value>>,
    parent_type_name: &str,
    buffer: &mut BytesMut,
    first: &mut bool,
) {
    for selection in selections {
        let field_val = obj
            .iter()
            .position(|(k, _)| k == &selection.response_key.as_str())
            .map(|idx| &obj[idx].1);
        let typename_field = field_val
            .and_then(|value| value.as_object())
            .and_then(|obj| {
                obj.iter()
                    .position(|(k, _)| k == &TYPENAME_FIELD_NAME)
                    .and_then(|idx| obj[idx].1.as_str())
            })
            .unwrap_or(&selection.field_type);

        let res = check(
            &selection.conditions,
            parent_type_name,
            typename_field,
            field_val,
            variable_values,
        );

        match res {
            Ok(_) => {
                if *first {
                    buffer.put(OPEN_BRACE);
                } else {
                    buffer.put(COMMA);
                }
                *first = false;

                buffer.put(QUOTE);
                buffer.put(selection.response_key.as_bytes());
                buffer.put(QUOTE);
                buffer.put(COLON);

                if let Some(field_val) = field_val {
                    project_selection_set(field_val, errors, selection, variable_values, buffer);
                } else if selection.field_name == TYPENAME_FIELD_NAME {
                    // If the field is TYPENAME_FIELD, we should set it to the parent type name
                    buffer.put(QUOTE);
                    buffer.put(parent_type_name.as_bytes());
                    buffer.put(QUOTE);
                } else {
                    // If the field is not found in the object, set it to Null
                    buffer.put(NULL);
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
                    buffer.put(OPEN_BRACE);
                } else {
                    buffer.put(COMMA);
                }
                *first = false;

                buffer.put(QUOTE);
                buffer.put(selection.response_key.as_bytes());
                buffer.put(QUOTE);
                buffer.put(COLON);
                buffer.put(NULL);
                errors.push(GraphQLError {
                    message: "Value is not a valid enum value".to_string(),
                    locations: None,
                    path: None,
                    extensions: None,
                });
            }
            Err(FieldProjectionConditionError::InvalidFieldType) => {
                if *first {
                    buffer.put(OPEN_BRACE);
                } else {
                    buffer.put(COMMA);
                }
                *first = false;

                // Skip this field as the field type does not match
                buffer.put(QUOTE);
                buffer.put(selection.response_key.as_bytes());
                buffer.put(QUOTE);
                buffer.put(COLON);
                buffer.put(NULL);
            }
        }
    }
}

fn check(
    cond: &FieldProjectionCondition,
    parent_type_name: &str,
    field_type_name: &str,
    field_value: Option<&Value>,
    variable_values: &Option<HashMap<String, sonic_rs::Value>>,
) -> Result<(), FieldProjectionConditionError> {
    match cond {
        FieldProjectionCondition::And(condition_a, condition_b) => check(
            condition_a,
            parent_type_name,
            field_type_name,
            field_value,
            variable_values,
        )
        .and(check(
            condition_b,
            parent_type_name,
            field_type_name,
            field_value,
            variable_values,
        )),
        FieldProjectionCondition::Or(condition_a, condition_b) => check(
            condition_a,
            parent_type_name,
            field_type_name,
            field_value,
            variable_values,
        )
        .or(check(
            condition_b,
            parent_type_name,
            field_type_name,
            field_value,
            variable_values,
        )),
        FieldProjectionCondition::IncludeIfVariable(variable_name) => {
            if let Some(values) = variable_values {
                if values
                    .get(variable_name)
                    .is_some_and(|v| v.as_bool().unwrap_or(false))
                {
                    Ok(())
                } else {
                    Err(FieldProjectionConditionError::Skip)
                }
            } else {
                Err(FieldProjectionConditionError::Skip)
            }
        }
        FieldProjectionCondition::SkipIfVariable(variable_name) => {
            if let Some(values) = variable_values {
                if values
                    .get(variable_name)
                    .is_some_and(|v| v.as_bool().unwrap_or(false))
                {
                    return Err(FieldProjectionConditionError::Skip);
                }
            }
            Ok(())
        }
        FieldProjectionCondition::ParentTypeCondition(type_condition) => {
            let is_valid = match type_condition {
                TypeCondition::Exact(expected_type) => parent_type_name == expected_type,
                TypeCondition::OneOf(possible_types) => possible_types.contains(parent_type_name),
            };
            if is_valid {
                Ok(())
            } else {
                Err(FieldProjectionConditionError::InvalidParentType)
            }
        }
        FieldProjectionCondition::FieldTypeCondition(type_condition) => {
            let is_valid = match type_condition {
                TypeCondition::Exact(expected_type) => field_type_name == expected_type,
                TypeCondition::OneOf(possible_types) => possible_types.contains(field_type_name),
            };

            if is_valid {
                Ok(())
            } else {
                Err(FieldProjectionConditionError::InvalidFieldType)
            }
        }
        FieldProjectionCondition::EnumValuesCondition(enum_values) => {
            if let Some(Value::String(string_value)) = field_value {
                if enum_values.contains(&string_value.to_string()) {
                    Ok(())
                } else {
                    Err(FieldProjectionConditionError::InvalidEnumValue)
                }
            } else {
                Ok(())
            }
        }
    }
}
