use crate::{projection::writer::ResponseWriter, response::value::Value};
use bytes::{Bytes, BytesMut};
use query_plan_executor::projection::{
    FieldProjectionCondition, FieldProjectionConditionError, FieldProjectionPlan,
};
use sonic_rs::{JsonValueTrait, LazyValue};
use std::collections::HashMap;

use tracing::{instrument, warn};

use crate::utils::consts::TYPENAME_FIELD_NAME;

#[instrument(level = "trace", skip_all)]
pub fn project_by_operation(
    data: &Value,
    errors: Vec<LazyValue>,
    extensions: &Option<HashMap<String, sonic_rs::Value>>,
    operation_type_name: &str,
    selections: &Vec<FieldProjectionPlan>,
    variable_values: &Option<HashMap<String, sonic_rs::Value>>,
) -> Bytes {
    let mut buffer = BytesMut::with_capacity(4096);
    let mut writer = ResponseWriter::new(&mut buffer);
    let mut errors = errors;

    writer.start_object();
    writer.write_key("data");

    if let Some(data_map) = data.as_object() {
        project_selection_set_with_map(
            data_map,
            &mut errors,
            selections,
            variable_values,
            operation_type_name,
            &mut writer,
        );
    } else {
        writer.write_null();
    }

    if !errors.is_empty() {
        writer.write_key("errors");
        writer.write_raw_slice(&sonic_rs::to_vec(&errors).unwrap());
    }

    if extensions.as_ref().is_some_and(|ext| !ext.is_empty()) {
        writer.write_key("extensions");
        writer.write_raw_slice(&sonic_rs::to_vec(extensions.as_ref().unwrap()).unwrap());
    }

    writer.end_object();
    buffer.freeze()
}

fn project_selection_set(
    data: &Value,
    errors: &mut Vec<LazyValue>,
    selection: &FieldProjectionPlan,
    variable_values: &Option<HashMap<String, sonic_rs::Value>>,
    writer: &mut ResponseWriter,
) {
    match data {
        Value::Null => writer.write_null(),
        Value::Bool(value) => writer.write_bool(*value),
        Value::U64(num) => writer.write_u64(*num),
        Value::I64(num) => writer.write_i64(*num),
        Value::F64(num) => writer.write_f64(*num),
        Value::String(value) => writer.write_string(value),
        Value::Array(arr) => {
            writer.start_array();
            let mut first = true;
            for item in arr.iter() {
                if !first {
                    writer.write_separator();
                }
                project_selection_set(item, errors, selection, variable_values, writer);
                first = false;
            }
            writer.end_array();
        }
        Value::Object(obj) => {
            match selection.selections.as_ref() {
                Some(selections) => {
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
                        writer,
                    );
                }
                None => {
                    // If the selection set is not projected, we should return null
                    writer.write_null();
                }
            }
        }
    };
}

// TODO: simplfy args
#[allow(clippy::too_many_arguments)]
fn project_selection_set_with_map(
    obj: &Vec<(&str, Value)>,
    errors: &mut Vec<LazyValue>,
    selections: &Vec<FieldProjectionPlan>,
    variable_values: &Option<HashMap<String, sonic_rs::Value>>,
    parent_type_name: &str,
    writer: &mut ResponseWriter,
) {
    writer.start_object();
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
                writer.write_key(&selection.response_key);

                if let Some(field_val) = field_val {
                    project_selection_set(field_val, errors, selection, variable_values, writer);
                } else if selection.field_name == TYPENAME_FIELD_NAME {
                    // If the field is TYPENAME_FIELD, we should set it to the parent type name
                    writer.write_string(parent_type_name);
                } else {
                    // If the field is not found in the object, set it to Null
                    writer.write_null();
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
                writer.write_key(&selection.response_key);
                writer.write_null();
                // errors.push(Value::);
                // errors.push(GraphQLError {
                //     message: "Value is not a valid enum value".to_string(),
                //     locations: None,
                //     path: None,
                //     extensions: None,
                // });
            }
            Err(FieldProjectionConditionError::InvalidFieldType) => {
                // Skip this field as the field type does not match
                writer.write_key(&selection.response_key);
                writer.write_null();
            }
        }
    }
    writer.end_object();
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
