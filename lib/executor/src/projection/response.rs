use crate::json_writer::BytesMutExt;
use crate::projection::error::ProjectionError;
use crate::projection::plan::{
    FieldProjectionCondition, FieldProjectionConditionError, FieldProjectionPlan,
};
use crate::response::graphql_error::GraphQLError;
use crate::response::value::Value;
use ntex_bytes::{Bytes as OutputBytes, BytesMut as OutputBytesMut};
use sonic_rs::JsonValueTrait;
use std::collections::HashMap;

use crate::utils::consts::{
    CLOSE_BRACE, CLOSE_BRACKET, COLON, COMMA, EMPTY_OBJECT, FALSE, NULL, OPEN_BRACE, OPEN_BRACKET,
    QUOTE, TRUE, TYPENAME_FIELD_NAME,
};
use tracing::{instrument, warn};

#[instrument(level = "trace", skip_all)]
pub fn project_by_operation(
    data: &Value,
    errors: Vec<GraphQLError>,
    extensions: &Option<HashMap<String, sonic_rs::Value>>,
    operation_type_name: &str,
    selections: &Vec<FieldProjectionPlan>,
    variable_values: &Option<HashMap<String, sonic_rs::Value>>,
) -> Result<OutputBytes, ProjectionError> {
    let mut buffer = OutputBytesMut::new();
    buffer.extend_from_slice(OPEN_BRACE);
    buffer.extend_from_slice(QUOTE);
    buffer.extend_from_slice("data".as_bytes());
    buffer.extend_from_slice(QUOTE);
    buffer.extend_from_slice(COLON);

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
            buffer.extend_from_slice(CLOSE_BRACE);
        } else {
            // If no selections were made, we should return an empty object
            buffer.extend_from_slice(EMPTY_OBJECT);
        }
    }

    if !errors.is_empty() {
        buffer.extend_from_slice(COMMA);
        buffer.extend_from_slice(QUOTE);
        buffer.extend_from_slice("errors".as_bytes());
        buffer.extend_from_slice(QUOTE);
        buffer.extend_from_slice(COLON);
        buffer.extend_from_slice(
            &sonic_rs::to_vec(&errors)
                .map_err(|e| ProjectionError::ErrorsSerializationFailure(e.to_string()))?,
        );
    }

    if let Some(ext) = extensions.as_ref() {
        if !ext.is_empty() {
            let serialized_extensions = sonic_rs::to_vec(ext)
                .map_err(|e| ProjectionError::ExtensionsSerializationFailure(e.to_string()))?;
            buffer.extend_from_slice(COMMA);
            buffer.extend_from_slice(QUOTE);
            buffer.extend_from_slice("extensions".as_bytes());
            buffer.extend_from_slice(QUOTE);
            buffer.extend_from_slice(COLON);
            buffer.extend_from_slice(&serialized_extensions);
        }
    }

    buffer.extend_from_slice(CLOSE_BRACE);
    Ok(buffer.freeze())
}

fn project_selection_set(
    data: &Value,
    errors: &mut Vec<GraphQLError>,
    selection: &FieldProjectionPlan,
    variable_values: &Option<HashMap<String, sonic_rs::Value>>,
    buffer: &mut OutputBytesMut,
) {
    match data {
        Value::Null => buffer.extend_from_slice(NULL),
        Value::Bool(true) => buffer.extend_from_slice(TRUE),
        Value::Bool(false) => buffer.extend_from_slice(FALSE),
        Value::U64(num) => buffer.write_u64(*num),
        Value::I64(num) => buffer.write_i64(*num),
        Value::F64(num) => buffer.write_f64(*num),
        Value::String(value) => buffer.write_and_escape_string(value),
        Value::Array(arr) => {
            buffer.extend_from_slice(OPEN_BRACKET);
            let mut first = true;
            for item in arr.iter() {
                if !first {
                    buffer.extend_from_slice(COMMA);
                }
                project_selection_set(item, errors, selection, variable_values, buffer);
                first = false;
            }
            buffer.extend_from_slice(CLOSE_BRACKET);
        }
        Value::Object(obj) => {
            match selection.selections.as_ref() {
                Some(selections) => {
                    let mut first = true;
                    let type_name = obj
                        .binary_search_by_key(&TYPENAME_FIELD_NAME, |(k, _)| k)
                        .ok()
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
                        buffer.extend_from_slice(CLOSE_BRACE);
                    } else {
                        // If no selections were made, we should return an empty object
                        buffer.extend_from_slice(EMPTY_OBJECT);
                    }
                }
                None => {
                    // If the selection set is not projected, we should return null
                    buffer.extend_from_slice(NULL)
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
    buffer: &mut OutputBytesMut,
    first: &mut bool,
) {
    for selection in selections {
        let field_val = obj
            .binary_search_by_key(&selection.field_name.as_str(), |(k, _)| *k)
            .ok()
            .or_else(|| {
                if selection.field_name == selection.response_key {
                    None
                } else {
                    obj.binary_search_by_key(&selection.response_key.as_str(), |(k, _)| *k)
                        .ok()
                }
            })
            .map(|idx| &obj[idx].1);

        match check(
            &selection.conditions,
            parent_type_name,
            &selection.field_type,
            field_val,
            variable_values,
        ) {
            Ok(_) => {
                if *first {
                    buffer.extend_from_slice(OPEN_BRACE);
                } else {
                    buffer.extend_from_slice(COMMA);
                }
                *first = false;

                buffer.extend_from_slice(QUOTE);
                buffer.extend_from_slice(selection.response_key.as_bytes());
                buffer.extend_from_slice(QUOTE);
                buffer.extend_from_slice(COLON);

                if let Some(field_val) = field_val {
                    project_selection_set(field_val, errors, selection, variable_values, buffer);
                } else if selection.field_name == TYPENAME_FIELD_NAME {
                    // If the field is TYPENAME_FIELD, we should set it to the parent type name
                    buffer.extend_from_slice(QUOTE);
                    buffer.extend_from_slice(parent_type_name.as_bytes());
                    buffer.extend_from_slice(QUOTE);
                } else {
                    // If the field is not found in the object, set it to Null
                    buffer.extend_from_slice(NULL);
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
                    buffer.extend_from_slice(OPEN_BRACE);
                } else {
                    buffer.extend_from_slice(COMMA);
                }
                *first = false;

                buffer.extend_from_slice(QUOTE);
                buffer.extend_from_slice(selection.response_key.as_bytes());
                buffer.extend_from_slice(QUOTE);
                buffer.extend_from_slice(COLON);
                buffer.extend_from_slice(NULL);
                errors.push(GraphQLError {
                    message: "Value is not a valid enum value".to_string(),
                    locations: None,
                    path: None,
                    extensions: None,
                });
            }
            Err(FieldProjectionConditionError::InvalidFieldType) => {
                if *first {
                    buffer.extend_from_slice(OPEN_BRACE);
                } else {
                    buffer.extend_from_slice(COMMA);
                }
                *first = false;

                // Skip this field as the field type does not match
                buffer.extend_from_slice(QUOTE);
                buffer.extend_from_slice(selection.response_key.as_bytes());
                buffer.extend_from_slice(QUOTE);
                buffer.extend_from_slice(COLON);
                buffer.extend_from_slice(NULL);
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
        FieldProjectionCondition::ParentTypeCondition(possible_types) => {
            if possible_types.contains(parent_type_name) {
                Ok(())
            } else {
                Err(FieldProjectionConditionError::InvalidParentType)
            }
        }
        FieldProjectionCondition::FieldTypeCondition(possible_types) => {
            let field_type_name = field_value
                .and_then(|value| value.as_object())
                .and_then(|obj| {
                    obj.binary_search_by_key(&TYPENAME_FIELD_NAME, |(k, _)| k)
                        .ok()
                        .and_then(|idx| obj[idx].1.as_str())
                })
                .unwrap_or(field_type_name);
            if possible_types.contains(field_type_name) {
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
