use std::collections::HashMap;

use hive_router_query_planner::state::supergraph_state::TypeNode;
use sonic_rs::{JsonNumberTrait, Value, ValueRef};

use crate::introspection::schema::SchemaMetadata;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum VariableCoercionError {
    #[error("Variable \"${name}\" of required type \"{type_name}\" was not provided.")]
    MissingNonNullableVariable { name: String, type_name: String },

    #[error("Expected value of non-null type \"{type_name}\" not to be null.")]
    UnexpectedNull { type_name: String },

    #[error("Value \"{value}\" does not exist in \"{type_name}\" enum.")]
    InvalidEnumValue { value: String, type_name: String },

    #[error("Enum \"{type_name}\" cannot represent non-string value: {value}.")]
    ExpectedEnumString { type_name: String, value: String },

    #[error("Expected value of type \"{type_name}\" to include required field \"{field_name}\".")]
    MissingField {
        field_name: String,
        type_name: String,
    },

    #[error("Expected value of type \"{type_name}\" to be an object, found: {value}.")]
    ExpectedObject { type_name: String, value: String },

    #[error("String cannot represent a non string value: {value}.")]
    ExpectedString { value: String },

    #[error("ID cannot represent value: {value}.")]
    ExpectedId { value: String },

    #[error("Int cannot represent non-integer value: {value}.")]
    ExpectedInteger { value: String },

    #[error("Float cannot represent non numeric value: {value}.")]
    ExpectedFloat { value: String },

    #[error("Boolean cannot represent a non boolean value: {value}.")]
    ExpectedBoolean { value: String },
}

#[inline]
pub fn collect_variables(
    operation: &hive_router_query_planner::ast::operation::OperationDefinition,
    variables_map: &mut HashMap<String, Value>,
    schema_metadata: &SchemaMetadata,
) -> Result<Option<HashMap<String, Value>>, VariableCoercionError> {
    if operation.variable_definitions.is_none() {
        return Ok(None);
    }
    let variable_definitions = operation.variable_definitions.as_ref().unwrap();

    let collected_variables: Result<Vec<Option<(String, Value)>>, VariableCoercionError> =
        variable_definitions
            .iter()
            .map(|variable_definition| {
                let variable_name = variable_definition.name.as_str();
                if let Some(variable_value) = variables_map.remove(variable_name) {
                    validate_runtime_value(
                        variable_value.as_ref(),
                        &variable_definition.variable_type,
                        schema_metadata,
                    )?;
                    return Ok(Some((variable_name.to_string(), variable_value)));
                }
                if let Some(default_value) = &variable_definition.default_value {
                    let default_value_coerced: Value = default_value.into();
                    validate_runtime_value(
                        default_value_coerced.as_ref(),
                        &variable_definition.variable_type,
                        schema_metadata,
                    )?;
                    return Ok(Some((variable_name.to_string(), default_value_coerced)));
                }
                if variable_definition.variable_type.is_non_null() {
                    return Err(VariableCoercionError::MissingNonNullableVariable {
                        name: variable_name.to_string(),
                        type_name: variable_definition.variable_type.to_string(),
                    });
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

#[inline]
fn validate_runtime_value(
    value: ValueRef,
    type_node: &TypeNode,
    schema_metadata: &SchemaMetadata,
) -> Result<(), VariableCoercionError> {
    if let ValueRef::Null = value {
        return if type_node.is_non_null() {
            Err(VariableCoercionError::UnexpectedNull {
                type_name: type_node.to_string(),
            })
        } else {
            Ok(())
        };
    }
    match type_node {
        TypeNode::Named(name) => {
            if let Some(enum_values) = schema_metadata.enum_values.get(name) {
                if let ValueRef::String(ref s) = value {
                    if !enum_values.contains(&s.to_string()) {
                        return Err(VariableCoercionError::InvalidEnumValue {
                            value: s.to_string(),
                            type_name: name.clone(),
                        });
                    }
                } else {
                    return Err(VariableCoercionError::ExpectedEnumString {
                        type_name: name.clone(),
                        value: format!("{:?}", value),
                    });
                }
            } else if let Some(fields) = schema_metadata.type_fields.get(name) {
                if let ValueRef::Object(obj) = value {
                    for (field_name, field_info) in fields {
                        if let Some(field_value) = obj.get(field_name) {
                            validate_runtime_value(
                                field_value.as_ref(),
                                &TypeNode::Named(field_info.output_type_name.to_string()),
                                schema_metadata,
                            )?;
                        } else {
                            return Err(VariableCoercionError::MissingField {
                                field_name: field_name.clone(),
                                type_name: name.clone(),
                            });
                        }
                    }
                } else {
                    return Err(VariableCoercionError::ExpectedObject {
                        type_name: name.clone(),
                        value: format!("{:?}", value),
                    });
                }
            } else {
                return match name.as_str() {
                    "String" => {
                        if let ValueRef::String(_) = value {
                            Ok(())
                        } else {
                            Err(VariableCoercionError::ExpectedString {
                                value: format!("{:?}", value),
                            })
                        }
                    }
                    "ID" => {
                        if let ValueRef::String(_) = value {
                            Ok(())
                        } else {
                            Err(VariableCoercionError::ExpectedId {
                                value: format!("{:?}", value),
                            })
                        }
                    }
                    "Int" => {
                        let is_valid = matches!(value, ValueRef::Number(ref num) if num.is_i64());
                        if is_valid {
                            Ok(())
                        } else {
                            Err(VariableCoercionError::ExpectedInteger {
                                value: format!("{:?}", value),
                            })
                        }
                    }
                    "Float" => {
                        let is_valid = matches!(
                            value,
                            ValueRef::Number(ref num) if num.is_f64() || num.is_i64()
                        );
                        if is_valid {
                            Ok(())
                        } else {
                            Err(VariableCoercionError::ExpectedFloat {
                                value: format!("{:?}", value),
                            })
                        }
                    }
                    "Boolean" => {
                        if let ValueRef::Bool(_) = value {
                            Ok(())
                        } else {
                            Err(VariableCoercionError::ExpectedBoolean {
                                value: format!("{:?}", value),
                            })
                        }
                    }
                    _ => Ok(()),
                };
            }
        }
        TypeNode::NonNull(inner_type) => {
            // The null check is now handled above, so we can just recurse.
            validate_runtime_value(value, inner_type, schema_metadata)?;
        }
        TypeNode::List(inner_type) => {
            if let ValueRef::Array(arr) = value {
                for item in arr.iter() {
                    validate_runtime_value(item.as_ref(), inner_type, schema_metadata)?;
                }
            } else {
                validate_runtime_value(value, inner_type, schema_metadata)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn allow_null_values_for_nullable_scalar_types() {
        let schema_metadata = crate::introspection::schema::SchemaMetadata::default();

        let scalars = vec!["String", "Int", "Float", "Boolean", "ID"];
        for scalar in scalars {
            let type_node = crate::variables::TypeNode::Named(scalar.to_string());

            let value = sonic_rs::ValueRef::Null;

            let result = super::validate_runtime_value(value, &type_node, &schema_metadata);
            assert_eq!(result, Ok(()));
        }
    }
    #[test]
    fn allow_null_values_for_nullable_list_types() {
        let schema_metadata = crate::introspection::schema::SchemaMetadata::default();
        let type_node = crate::variables::TypeNode::List(Box::new(
            crate::variables::TypeNode::Named("String".to_string()),
        ));
        let value = sonic_rs::ValueRef::Null;
        let result = super::validate_runtime_value(value, &type_node, &schema_metadata);
        assert_eq!(result, Ok(()));
    }
    #[test]
    fn allow_matching_non_list_values_for_list_types() {
        let schema_metadata = crate::introspection::schema::SchemaMetadata::default();
        let type_node = crate::variables::TypeNode::List(Box::new(
            crate::variables::TypeNode::Named("String".to_string()),
        ));
        let value = sonic_rs::ValueRef::String("not a list");
        let result = super::validate_runtime_value(value, &type_node, &schema_metadata);
        assert_eq!(result, Ok(()));
    }
    #[test]
    fn disallow_non_matching_non_list_values_for_list_types() {
        let schema_metadata = crate::introspection::schema::SchemaMetadata::default();
        let type_node = crate::variables::TypeNode::List(Box::new(
            crate::variables::TypeNode::Named("String".to_string()),
        ));
        let value = sonic_rs::ValueRef::Number(sonic_rs::Number::from(123));
        let result = super::validate_runtime_value(value, &type_node, &schema_metadata);
        assert!(result.is_err());
    }
}
