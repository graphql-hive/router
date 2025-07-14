use hashbrown::HashMap;

use query_planner::state::supergraph_state::TypeNode;
use serde_json::Value;

use crate::schema_metadata::SchemaMetadata;

pub fn collect_variables(
    operation: &query_planner::ast::operation::OperationDefinition,
    variables: &Option<HashMap<String, Value>>,
    schema_metadata: &SchemaMetadata,
) -> Result<Option<HashMap<String, Value>>, String> {
    if operation.variable_definitions.is_none() {
        return Ok(None);
    }
    let variable_definitions = operation.variable_definitions.as_ref().unwrap();
    let collected_variables: Result<Vec<Option<(String, Value)>>, String> = variable_definitions
        .iter()
        .map(|variable_definition| {
            let variable_name = variable_definition.name.to_string();
            if let Some(variable_value) = variables.as_ref().and_then(|v| v.get(&variable_name)) {
                validate_runtime_value(
                    variable_value,
                    &variable_definition.variable_type,
                    schema_metadata,
                )?;
                return Ok(Some((variable_name, variable_value.clone())));
            }
            if let Some(default_value) = &variable_definition.default_value {
                // Assuming value_from_ast now returns Result<Value, String> or similar
                // and needs to be adapted if it returns Option or panics.
                // For now, let's assume it can return an Err that needs to be propagated.
                let default_value_coerced: Value = default_value.into();
                validate_runtime_value(
                    &default_value_coerced,
                    &variable_definition.variable_type,
                    schema_metadata,
                )?;
                return Ok(Some((variable_name, default_value_coerced)));
            }
            if variable_definition.variable_type.is_non_null() {
                return Err(format!(
                    "Variable '{}' is non-nullable but no value was provided",
                    variable_name
                ));
            }
            Ok(None)
        })
        .collect();

    let variable_values: HashMap<String, Value> =
        collected_variables?.into_iter().flatten().collect();

    if variable_values.is_empty() {
        Ok(None)
    } else {
        Ok(Some(variable_values))
    }
}

fn validate_runtime_value(
    value: &Value,
    type_node: &TypeNode,
    schema_metadata: &SchemaMetadata,
) -> Result<(), String> {
    match type_node {
        TypeNode::Named(name) => {
            if let Some(enum_values) = schema_metadata.enum_values.get(name) {
                if let Value::String(ref s) = value {
                    if !enum_values.contains(&s.to_string()) {
                        return Err(format!(
                            "Value '{}' is not a valid enum value for type '{}'",
                            s, name
                        ));
                    }
                } else {
                    return Err(format!(
                        "Expected a string for enum type '{}', got {:?}",
                        name, value
                    ));
                }
            } else if let Some(fields) = schema_metadata.type_fields.get(name) {
                if let Value::Object(obj) = value {
                    for (field_name, field_type) in fields {
                        if let Some(field_value) = obj.get(field_name) {
                            validate_runtime_value(
                                field_value,
                                &TypeNode::Named(field_type.to_string()),
                                schema_metadata,
                            )?;
                        } else {
                            return Err(format!(
                                "Missing field '{}' for type '{}'",
                                field_name, name
                            ));
                        }
                    }
                } else {
                    return Err(format!(
                        "Expected an object for type '{}', got {:?}",
                        name, value
                    ));
                }
            } else {
                return match name.as_str() {
                    "String" => {
                        if let Value::String(_) = value {
                            Ok(())
                        } else {
                            Err(format!(
                                "Expected a string for type '{}', got {:?}",
                                name, value
                            ))
                        }
                    }
                    "Int" => {
                        if let Value::Number(num) = value {
                            if num.is_i64() {
                                Ok(())
                            } else {
                                Err(format!(
                                    "Expected an integer for type '{}', got {:?}",
                                    name, value
                                ))
                            }
                        } else {
                            Err(format!(
                                "Expected a number for type '{}', got {:?}",
                                name, value
                            ))
                        }
                    }
                    "Float" => {
                        if let Value::Number(num) = value {
                            if num.is_f64() || num.is_i64() {
                                Ok(())
                            } else {
                                Err(format!(
                                    "Expected a float for type '{}', got {:?}",
                                    name, value
                                ))
                            }
                        } else {
                            Err(format!(
                                "Expected a number for type '{}', got {:?}",
                                name, value
                            ))
                        }
                    }
                    "Boolean" => {
                        if let Value::Bool(_) = value {
                            Ok(())
                        } else {
                            Err(format!(
                                "Expected a boolean for type '{}', got {:?}",
                                name, value
                            ))
                        }
                    }
                    "ID" => {
                        if let Value::String(_) = value {
                            Ok(())
                        } else {
                            Err(format!("Expected a string for type 'ID', got {:?}", value))
                        }
                    }
                    _ => Ok(()),
                };
            }
        }
        TypeNode::NonNull(inner_type) => {
            if value.is_null() {
                return Err("Value cannot be null for non-nullable type".to_string());
            }
            validate_runtime_value(value, inner_type, schema_metadata)?;
        }
        TypeNode::List(inner_type) => {
            if let Value::Array(arr) = value {
                for item in arr {
                    validate_runtime_value(item, inner_type, schema_metadata)?;
                }
            } else {
                return Err(format!("Expected an array for list type, got {:?}", value));
            }
        }
    }
    Ok(())
}
