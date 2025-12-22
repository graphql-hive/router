use crate::projection::error::ProjectionError;
use crate::projection::plan::{
    FieldProjectionCondition, FieldProjectionConditionError, FieldProjectionPlan,
    ProjectionValueSource, TypeCondition,
};
use crate::response::graphql_error::{GraphQLError, GraphQLErrorExtensions};
use crate::response::value::Value;
use bytes::BufMut;
use indexmap::IndexMap;
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
    selections: &IndexMap<String, FieldProjectionPlan>,
    variable_values: &Option<HashMap<String, sonic_rs::Value>>,
    response_size_estimate: usize,
) -> Result<Vec<u8>, ProjectionError> {
    let mut buffer = Vec::with_capacity(response_size_estimate);
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
    } else {
        buffer.put(NULL);
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
    Ok(buffer)
}

fn project_without_selection_set(data: &Value, buffer: &mut Vec<u8>) {
    match data {
        Value::Null => buffer.put(NULL),
        Value::Bool(true) => buffer.put(TRUE),
        Value::Bool(false) => buffer.put(FALSE),
        Value::U64(num) => write_u64(buffer, *num),
        Value::I64(num) => write_i64(buffer, *num),
        Value::F64(num) => write_f64(buffer, *num),
        Value::String(value) => write_and_escape_string(buffer, value),
        Value::Object(value) => {
            buffer.put(OPEN_BRACE);
            let mut first = true;
            for (key, val) in value.iter() {
                if !first {
                    buffer.put(COMMA);
                }
                write_and_escape_string(buffer, key);
                buffer.put(COLON);
                project_without_selection_set(val, buffer);
                first = false;
            }
            buffer.put(CLOSE_BRACE);
        }
        Value::Array(arr) => {
            buffer.put(OPEN_BRACKET);
            let mut first = true;
            for item in arr.iter() {
                if !first {
                    buffer.put(COMMA);
                }
                project_without_selection_set(item, buffer);
                first = false;
            }
            buffer.put(CLOSE_BRACKET);
        }
    };
}

fn project_selection_set(
    data: &Value,
    errors: &mut Vec<GraphQLError>,
    selection: &FieldProjectionPlan,
    variable_values: &Option<HashMap<String, sonic_rs::Value>>,
    buffer: &mut Vec<u8>,
) {
    match data {
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
            match &selection.value {
                ProjectionValueSource::ResponseData {
                    selections: Some(selections),
                } => {
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
                ProjectionValueSource::ResponseData { selections: None } => {
                    // If the selection has no sub-selections, we serialize the whole object
                    project_without_selection_set(data, buffer);
                }
                ProjectionValueSource::Null => {
                    // This should not happen as we are in an object case, but just in case
                    buffer.put(NULL);
                }
            }
        }
        _ => {
            // If the data is not an object or array, we serialize it directly
            project_without_selection_set(data, buffer);
        }
    };
}

// TODO: simplfy args
#[allow(clippy::too_many_arguments)]
fn project_selection_set_with_map(
    obj: &Vec<(&str, Value)>,
    errors: &mut Vec<GraphQLError>,
    plans: &IndexMap<String, FieldProjectionPlan>,
    variable_values: &Option<HashMap<String, sonic_rs::Value>>,
    parent_type_name: &str,
    buffer: &mut Vec<u8>,
    first: &mut bool,
) {
    for (_, plan) in plans {
        let field_val = obj
            .iter()
            .position(|(k, _)| k == &plan.response_key.as_str())
            .map(|idx| &obj[idx].1);
        let typename_field = field_val
            .and_then(|value| value.as_object())
            .and_then(|obj| {
                obj.iter()
                    .position(|(k, _)| k == &TYPENAME_FIELD_NAME)
                    .and_then(|idx| obj[idx].1.as_str())
            })
            .unwrap_or(&plan.field_type);

        let res = check(
            &plan.conditions,
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
                buffer.put(plan.response_key.as_bytes());
                buffer.put(QUOTE);
                buffer.put(COLON);

                match &plan.value {
                    ProjectionValueSource::Null => {
                        buffer.put(NULL);
                        continue;
                    }
                    ProjectionValueSource::ResponseData { .. } => {
                        if let Some(field_val) = field_val {
                            project_selection_set(field_val, errors, plan, variable_values, buffer);
                        } else if plan.field_name == TYPENAME_FIELD_NAME {
                            // If the field is TYPENAME_FIELD, we should set it to the parent type name
                            buffer.put(QUOTE);
                            buffer.put(parent_type_name.as_bytes());
                            buffer.put(QUOTE);
                        } else {
                            // If the field is not found in the object, set it to Null
                            buffer.put(NULL);
                        }
                    }
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
                buffer.put(plan.response_key.as_bytes());
                buffer.put(QUOTE);
                buffer.put(COLON);
                buffer.put(NULL);
                errors.push(GraphQLError {
                    message: "Value is not a valid enum value".to_string(),
                    locations: None,
                    path: None,
                    extensions: GraphQLErrorExtensions::default(),
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
                buffer.put(plan.response_key.as_bytes());
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

#[cfg(test)]
mod tests {
    use graphql_parser::query::Definition;
    use hive_router_query_planner::{
        ast::{document::NormalizedDocument, normalization::create_normalized_document},
        consumer_schema::ConsumerSchema,
        utils::parsing::parse_operation,
    };
    use sonic_rs::json;

    use crate::{
        introspection::schema::SchemaWithMetadata,
        projection::{plan::FieldProjectionPlan, response::project_by_operation},
        response::value::Value,
    };

    #[test]
    fn project_scalars_with_object_value() {
        let supergraph = hive_router_query_planner::utils::parsing::parse_schema(
            r#"
            type Query {
                metadatas: Metadata!
            }

            scalar JSON

            type Metadata {
                id: ID!
                timestamp: String!
                data: JSON
            }
        "#,
        );
        let consumer_schema = ConsumerSchema::new_from_supergraph(&supergraph);
        let schema_metadata = consumer_schema.schema_metadata();
        let mut operation = parse_operation(
            r#"
            query GetMetadata {
                metadatas {
                    id
                    data
                }
            }
            "#,
        );
        let operation_ast = operation
            .definitions
            .iter_mut()
            .find_map(|def| match def {
                Definition::Operation(op) => Some(op),
                _ => None,
            })
            .unwrap();
        let normalized_operation: NormalizedDocument =
            create_normalized_document(operation_ast.clone(), Some("GetMetadata"));
        let (operation_type_name, selections) =
            FieldProjectionPlan::from_operation(&normalized_operation.operation, &schema_metadata);
        let data_json = json!({
            "__typename": "Query",
            "metadatas": [
                {
                    "__typename": "Metadata",
                    "id": "meta1",
                    "timestamp": "2024-01-01T00:00:00Z",
                    "data": {
                        "float": 41.5,
                        "int": -42,
                        "str": "value1",
                        "unsigned": 123,
                    }
                },
                {
                    "__typename": "Metadata",
                    "id": "meta2",
                    "data": null
                }
            ]
        });
        let data = Value::from(data_json.as_ref());
        let projection = project_by_operation(
            &data,
            vec![],
            &None,
            operation_type_name,
            &selections,
            &None,
            1000,
        );
        let projected_bytes = projection.unwrap();
        let projected_str = String::from_utf8(projected_bytes).unwrap();
        let expected_response = r#"{"data":{"metadatas":[{"id":"meta1","data":{"float":41.5,"int":-42,"str":"value1","unsigned":123}},{"id":"meta2","data":null}]}}"#;
        assert_eq!(projected_str, expected_response);
    }
}
