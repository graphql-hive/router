use crate::projection::error::ProjectionError;
use crate::projection::plan::{
    FieldProjectionCondition, FieldProjectionConditionError, FieldProjectionPlan,
    ProjectionValueSource,
};
use crate::response::graphql_error::GraphQLError;
use crate::response::value::Value;
use bytes::BufMut;
use sonic_rs::JsonValueTrait;
use std::cell::OnceCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::introspection::schema::SchemaMetadata;
use crate::json_writer::{write_and_escape_string, write_f64, write_i64, write_u64};
use crate::utils::consts::{
    CLOSE_BRACE, CLOSE_BRACKET, COLON, COMMA, EMPTY_OBJECT, FALSE, NULL, OPEN_BRACE, OPEN_BRACKET,
    QUOTE, TRUE, TYPENAME_FIELD_NAME,
};

/// Represents a type's name that can be either already resolved or lazily computed.
/// This avoids computing the type name when it's not needed, which is important for performance.
///
/// The enum is recursive - a Deferred variant can contain another TypeName as its parent,
/// creating a lazy chain that only resolves when actually needed.
#[derive(Clone)]
enum TypeName<'a> {
    Resolved(&'a str),
    Deferred {
        selection: &'a FieldProjectionPlan,
        data: Option<&'a Value<'a>>,
        parent: Rc<TypeName<'a>>,
        schema: &'a SchemaMetadata,
        /// Cache for the resolved type name to avoid recomputation
        cached: OnceCell<Result<&'a str, ProjectionError>>,
    },
}

impl<'a> TypeName<'a> {
    #[inline]
    fn resolved(type_name: &'a str) -> Self {
        TypeName::Resolved(type_name)
    }

    #[inline]
    fn deferred(
        selection: &'a FieldProjectionPlan,
        data: Option<&'a Value>,
        parent: TypeName<'a>,
        schema: &'a SchemaMetadata,
    ) -> Self {
        TypeName::Deferred {
            selection,
            data,
            parent: Rc::new(parent),
            schema,
            cached: OnceCell::new(),
        }
    }

    #[inline]
    fn get(&self) -> Result<&'a str, ProjectionError> {
        match self {
            TypeName::Resolved(name) => Ok(name),
            TypeName::Deferred {
                selection,
                data,
                parent,
                schema,
                cached,
            } => cached
                .get_or_init(|| resolve_type_name(selection, *data, parent, schema))
                .clone(),
        }
    }
}

// TODO: simplfy args
#[allow(clippy::too_many_arguments)]
pub fn project_by_operation(
    data: &Value,
    errors: Vec<GraphQLError>,
    extensions: &HashMap<String, sonic_rs::Value>,
    operation_type_name: &str,
    selections: &[FieldProjectionPlan],
    variable_values: &Option<HashMap<String, sonic_rs::Value>>,
    response_size_estimate: usize,
    schema_metadata: &SchemaMetadata,
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
            TypeName::resolved(operation_type_name),
            &mut buffer,
            &mut first,
            schema_metadata,
        )?;
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

    if !extensions.is_empty() {
        let serialized_extensions = sonic_rs::to_vec(extensions)
            .map_err(|e| ProjectionError::ExtensionsSerializationFailure(e.to_string()))?;
        buffer.put(COMMA);
        buffer.put(QUOTE);
        buffer.put("extensions".as_bytes());
        buffer.put(QUOTE);
        buffer.put(COLON);
        buffer.put_slice(&serialized_extensions);
    }

    buffer.put(CLOSE_BRACE);
    Ok(buffer)
}

pub fn serialize_value_to_buffer(data: &Value, buffer: &mut Vec<u8>) {
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
                serialize_value_to_buffer(val, buffer);
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
                serialize_value_to_buffer(item, buffer);
                first = false;
            }
            buffer.put(CLOSE_BRACKET);
        }
    };
}

fn project_selection_set<'a>(
    data: &'a Value,
    errors: &mut Vec<GraphQLError>,
    selection: &'a FieldProjectionPlan,
    variable_values: &Option<HashMap<String, sonic_rs::Value>>,
    buffer: &mut Vec<u8>,
    parent_type_name: TypeName<'a>,
    schema_metadata: &'a SchemaMetadata,
) -> Result<(), ProjectionError> {
    match data {
        Value::Array(arr) => {
            buffer.put(OPEN_BRACKET);
            let mut first = true;
            for item in arr.iter() {
                if !first {
                    buffer.put(COMMA);
                }
                project_selection_set(
                    item,
                    errors,
                    selection,
                    variable_values,
                    buffer,
                    parent_type_name.clone(),
                    schema_metadata,
                )?;
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
                    let type_name = TypeName::deferred(
                        selection,
                        Some(data),
                        parent_type_name,
                        schema_metadata,
                    );
                    project_selection_set_with_map(
                        obj,
                        errors,
                        selections,
                        variable_values,
                        type_name,
                        buffer,
                        &mut first,
                        schema_metadata,
                    )?;
                    if !first {
                        buffer.put(CLOSE_BRACE);
                    } else {
                        // If no selections were made, we should return an empty object
                        buffer.put(EMPTY_OBJECT);
                    }
                }
                ProjectionValueSource::ResponseData { selections: None } => {
                    // If the selection has no sub-selections, we serialize the whole object
                    serialize_value_to_buffer(data, buffer);
                }
                ProjectionValueSource::Null => {
                    // This should not happen as we are in an object case, but just in case
                    buffer.put(NULL);
                }
            }
        }
        _ => {
            // If the data is not an object or array, we serialize it directly
            serialize_value_to_buffer(data, buffer);
        }
    };
    Ok(())
}

// TODO: simplfy args
#[allow(clippy::too_many_arguments)]
fn project_selection_set_with_map<'a>(
    obj: &'a [(&str, Value)],
    errors: &mut Vec<GraphQLError>,
    plans: &'a [FieldProjectionPlan],
    variable_values: &Option<HashMap<String, sonic_rs::Value>>,
    parent_type_name: TypeName<'a>,
    buffer: &mut Vec<u8>,
    first: &mut bool,
    schema_metadata: &'a SchemaMetadata,
) -> Result<(), ProjectionError> {
    for plan in plans {
        if let Some(guard) = &plan.parent_type_guard {
            let name = parent_type_name.get()?;
            if !guard.matches(name) {
                // Seems like the field projection plan applies to other types, so move to the next one
                continue;
            }
        }

        let field_val = obj
            .binary_search_by_key(&plan.response_key.as_str(), |(k, _)| *k)
            .ok()
            .map(|idx| &obj[idx].1);

        let res = if let Some(conditions) = &plan.conditions {
            let field_type_name_cell = OnceCell::new();
            let field_type_name_fn = || {
                field_type_name_cell
                    .get_or_init(|| {
                        resolve_type_name(plan, field_val, &parent_type_name, schema_metadata)
                    })
                    .clone()
            };
            let parent_type_name_fn = || parent_type_name.get();
            check(
                conditions,
                &parent_type_name_fn,
                &field_type_name_fn,
                field_val,
                variable_values,
            )
        } else {
            Ok(())
        };

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
                        if plan.is_typename {
                            // If the field is TYPENAME_FIELD, we should set it to the parent type name
                            buffer.put(QUOTE);
                            buffer.put(parent_type_name.get()?.as_bytes());
                            buffer.put(QUOTE);
                        } else if let Some(field_val) = field_val {
                            project_selection_set(
                                field_val,
                                errors,
                                plan,
                                variable_values,
                                buffer,
                                parent_type_name.clone(),
                                schema_metadata,
                            )?;
                        } else {
                            // If the field is not found in the object, set it to Null
                            buffer.put(NULL);
                        }
                    }
                }
            }
            Err(FieldProjectionConditionError::Fatal(err)) => {
                return Err(err);
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
                errors.push(GraphQLError::from("Value is not a valid enum value"));
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
    Ok(())
}

#[inline]
fn check<'a, F, T>(
    cond: &FieldProjectionCondition,
    parent_type_name: &T,
    field_type_name: &F,
    field_value: Option<&Value>,
    variable_values: &Option<HashMap<String, sonic_rs::Value>>,
) -> Result<(), FieldProjectionConditionError>
where
    F: Fn() -> Result<&'a str, ProjectionError>,
    T: Fn() -> Result<&'a str, ProjectionError>,
{
    match cond {
        FieldProjectionCondition::And(condition_a, condition_b) => check(
            condition_a,
            parent_type_name,
            field_type_name,
            field_value,
            variable_values,
        )
        .and_then(|_| {
            check(
                condition_b,
                parent_type_name,
                field_type_name,
                field_value,
                variable_values,
            )
        }),
        FieldProjectionCondition::Or(condition_a, condition_b) => check(
            condition_a,
            parent_type_name,
            field_type_name,
            field_value,
            variable_values,
        )
        .or_else(|_| {
            check(
                condition_b,
                parent_type_name,
                field_type_name,
                field_value,
                variable_values,
            )
        }),
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
            if type_condition.matches(parent_type_name()?) {
                Ok(())
            } else {
                Err(FieldProjectionConditionError::InvalidParentType)
            }
        }
        FieldProjectionCondition::FieldTypeCondition(type_condition) => {
            if type_condition.matches(field_type_name()?) {
                Ok(())
            } else {
                Err(FieldProjectionConditionError::InvalidFieldType)
            }
        }
        FieldProjectionCondition::EnumValuesCondition(enum_values) => {
            if let Some(Value::String(string_value)) = field_value {
                if enum_values.contains(string_value.as_ref()) {
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

#[inline]
/// When an error is returned, it means a broken logic or state.
/// A scenario when a type is missing or a type is missing a field,
/// can only happen when field's projection rule lack a proper type guard,
/// or the type guard was not correctly enforced, resulting in applying a plan for a different parent type.
fn resolve_type_name<'a>(
    plan: &'a FieldProjectionPlan,
    field_val: Option<&'a Value>,
    parent_type_name: &TypeName<'a>,
    schema_metadata: &'a SchemaMetadata,
) -> Result<&'a str, ProjectionError> {
    if plan.is_typename {
        return Ok("String");
    }

    let typename_field = field_val
        .and_then(|value| value.as_object())
        .and_then(|obj| {
            obj.binary_search_by_key(&TYPENAME_FIELD_NAME, |(k, _)| *k)
                .ok()
                .and_then(|idx| obj[idx].1.as_str())
        });

    if let Some(typename) = typename_field {
        return Ok(typename);
    }

    let parent_type_name = parent_type_name.get()?;

    let fields = schema_metadata
        .get_type_fields(parent_type_name)
        .ok_or_else(|| ProjectionError::MissingType(parent_type_name.to_string()))?;

    fields
        .get(&plan.field_name)
        .map(|field_info| field_info.output_type_name.as_str())
        .ok_or_else(|| ProjectionError::MissingField {
            field_name: plan.field_name.to_string(),
            type_name: parent_type_name.to_string(),
        })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use graphql_tools::parser::query::Definition;
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
            create_normalized_document(operation_ast.clone(), Some("GetMetadata".into()));
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
            &HashMap::new(),
            operation_type_name,
            &selections,
            &None,
            1000,
            &schema_metadata,
        );
        let projected_bytes = projection.unwrap();
        let projected_str = String::from_utf8(projected_bytes).unwrap();
        let expected_response = r#"{"data":{"metadatas":[{"id":"meta1","data":{"float":41.5,"int":-42,"str":"value1","unsigned":123}},{"id":"meta2","data":null}]}}"#;
        assert_eq!(projected_str, expected_response);
    }

    #[test]
    fn test_duplicate_selections_in_merged_plans() {
        let supergraph = hive_router_query_planner::utils::parsing::parse_schema(
            r#"
              interface Node {
                id: ID!
              }

              type A implements Node {
                id: ID
                children: [AChild]
              }
              type B implements Node {
                id: ID!
                children: [BChild]
              }

              type AChild {
                id: ID
              }
              type BChild {
                id: ID
              }

              type Container {
                node: Node
              }
              type Query {
                nodes: [Container]
              }
        "#,
        );
        let consumer_schema = ConsumerSchema::new_from_supergraph(&supergraph);
        let schema_metadata = consumer_schema.schema_metadata();

        let mut operation = parse_operation(
            r#"
              query {
                nodes {
                  node {
                    ... on A {
                      children {
                        id
                      }
                    }
                    ...on B {
                      children {
                        id
                      }
                    }
                  }
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
            create_normalized_document(operation_ast.clone(), Some("SearchQuery".into()));
        let (operation_type_name, selections) =
            FieldProjectionPlan::from_operation(&normalized_operation.operation, &schema_metadata);

        let data_json = json!({
            "__typename": "Query",
            "nodes": [
                {
                    "node": {
                        "__typename": "A",
                        "children": []
                    }
                },
                {
                    "node": {
                        "__typename": "B",
                        "children": [
                            { "id": "b_child_1" }
                        ]
                    }
                }
            ]
        });
        let data = Value::from(data_json.as_ref());
        let projection = project_by_operation(
            &data,
            vec![],
            &HashMap::new(),
            operation_type_name,
            &selections,
            &None,
            1000,
            &schema_metadata,
        );
        let projected_bytes = projection.unwrap();
        let projected_value: sonic_rs::Value = sonic_rs::from_slice(&projected_bytes).unwrap();
        let projected_str = sonic_rs::to_string_pretty(&projected_value).unwrap();
        insta::assert_snapshot!(projected_str, @r#"
        {
          "data": {
            "nodes": [
              {
                "node": {
                  "children": []
                }
              },
              {
                "node": {
                  "children": [
                    {
                      "id": "b_child_1"
                    }
                  ]
                }
              }
            ]
          }
        }
        "#);
    }
}
