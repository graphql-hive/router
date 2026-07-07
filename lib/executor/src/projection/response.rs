use crate::execution::plan::ExecutionResultExtensions;
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

use crate::introspection::schema::{FieldNullability, SchemaMetadata};
use crate::json_writer::{write_and_escape_string, write_f64, write_i64, write_u64};
use crate::utils::consts::{
    CLOSE_BRACE, CLOSE_BRACKET, COLON, COMMA, EMPTY_OBJECT, FALSE, NULL, OPEN_BRACE, OPEN_BRACKET,
    QUOTE, TRUE, TYPENAME_FIELD_NAME,
};

enum NullPropagationDecision {
    /// An indicator that the `null` value should be propagated, since the field is non-null
    PropagateNullValue,
    /// An indicator that the `null` value should be kept as-is, since the field is nullable.
    KeepNullValue,
}

impl NullPropagationDecision {
    #[inline]
    fn should_propagate(&self) -> bool {
        matches!(self, NullPropagationDecision::PropagateNullValue)
    }
}

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
        /// Cache for the resolved type name to avoid recomputation
        cached: OnceCell<&'a str>,
    },
}

impl<'a> TypeName<'a> {
    #[inline]
    fn resolved(type_name: &'a str) -> Self {
        TypeName::Resolved(type_name)
    }

    #[inline]
    fn deferred(selection: &'a FieldProjectionPlan, data: Option<&'a Value>) -> Self {
        TypeName::Deferred {
            selection,
            data,
            cached: OnceCell::new(),
        }
    }

    #[inline]
    fn get(&self) -> &'a str {
        match self {
            TypeName::Resolved(name) => name,
            TypeName::Deferred {
                selection,
                data,
                cached,
            } => cached.get_or_init(|| resolve_type_name(selection, *data)),
        }
    }
}

// TODO: simplfy args
#[allow(clippy::too_many_arguments)]
pub fn project_by_operation(
    data: &Value,
    errors: Vec<GraphQLError>,
    extensions: &ExecutionResultExtensions<'_>,
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
        let null_propagation_checkpoint = buffer.len();
        // Start with first as true to add the opening brace
        let mut first = true;
        let use_simple_projection = is_simple_projection_set(selections);
        let null_propagation_decision = if use_simple_projection {
            project_selection_set_with_map_simple(data_map, selections, &mut buffer)
        } else {
            project_selection_set_with_map(
                data_map,
                &mut errors,
                selections,
                variable_values,
                TypeName::resolved(operation_type_name),
                &mut buffer,
                &mut first,
                schema_metadata,
            )?
        };

        if null_propagation_decision.should_propagate() {
            buffer.truncate(null_propagation_checkpoint);
            buffer.put(NULL);
        } else if use_simple_projection {
            // The simple path writes the complete object, including the closing brace.
        } else if !first {
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
        Value::RawJson(raw) => buffer.put_slice(raw.as_bytes()),
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

#[allow(clippy::too_many_arguments)]
fn project_selection_set<'a>(
    data: &'a Value,
    errors: &mut Vec<GraphQLError>,
    selection: &'a FieldProjectionPlan,
    variable_values: &Option<HashMap<String, sonic_rs::Value>>,
    buffer: &mut Vec<u8>,
    parent_type_name: TypeName<'a>,
    schema_metadata: &'a SchemaMetadata,
    nullability: &'a FieldNullability,
) -> Result<NullPropagationDecision, ProjectionError> {
    match data {
        Value::Array(arr) => {
            let null_propagation_checkpoint = buffer.len();
            let list_item_nullability = nullability.list_item();
            let item_non_null = list_item_nullability.is_some_and(FieldNullability::is_non_null);
            buffer.put(OPEN_BRACKET);
            let mut first = true;
            for item in arr.iter() {
                if !first {
                    buffer.put(COMMA);
                }
                let needs_null_propagation = project_selection_set(
                    item,
                    errors,
                    selection,
                    variable_values,
                    buffer,
                    parent_type_name.clone(),
                    schema_metadata,
                    list_item_nullability.unwrap_or(nullability),
                )?;

                // A `null` at a Non-Null element of this list propagates to the list itself.
                if needs_null_propagation.should_propagate() && item_non_null {
                    buffer.truncate(null_propagation_checkpoint);
                    buffer.put(NULL);
                    return Ok(NullPropagationDecision::PropagateNullValue);
                }

                first = false;
            }

            buffer.put(CLOSE_BRACKET);
            Ok(NullPropagationDecision::KeepNullValue)
        }
        Value::Object(obj) => {
            match &selection.value {
                ProjectionValueSource::ResponseData {
                    selections: Some(selections),
                } => {
                    let null_propagation_checkpoint = buffer.len();
                    let null_propagation_decision = if selection.simple_projection {
                        project_selection_set_with_map_simple(obj, selections, buffer)
                    } else {
                        let mut first = true;
                        let type_name = if selection.subtree_needs_type_name {
                            TypeName::deferred(selection, Some(data))
                        } else {
                            parent_type_name
                        };
                        let null_propagation_decision = project_selection_set_with_map(
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

                        null_propagation_decision
                    };

                    if null_propagation_decision.should_propagate() {
                        buffer.truncate(null_propagation_checkpoint);
                        buffer.put(NULL);
                        return Ok(NullPropagationDecision::PropagateNullValue);
                    }

                    Ok(NullPropagationDecision::KeepNullValue)
                }
                ProjectionValueSource::ResponseData { selections: None } => {
                    // If the selection has no sub-selections, we serialize the whole object
                    serialize_value_to_buffer(data, buffer);
                    Ok(NullPropagationDecision::KeepNullValue)
                }
                ProjectionValueSource::Null => {
                    // This should not happen as we are in an object case, but just in case
                    buffer.put(NULL);
                    Ok(NullPropagationDecision::PropagateNullValue)
                }
            }
        }
        Value::Null => {
            buffer.put(NULL);
            Ok(NullPropagationDecision::PropagateNullValue)
        }
        _ => {
            // If the data is not an object or array, we serialize it directly
            serialize_value_to_buffer(data, buffer);
            Ok(NullPropagationDecision::KeepNullValue)
        }
    }
}

#[inline]
fn is_simple_projection_set(plans: &[FieldProjectionPlan]) -> bool {
    plans.iter().all(|plan| plan.simple_projection)
}

fn project_selection_set_with_map_simple(
    obj: &[(&str, Value)],
    plans: &[FieldProjectionPlan],
    buffer: &mut Vec<u8>,
) -> NullPropagationDecision {
    buffer.put(OPEN_BRACE);
    let mut field_lookup_cursor = 0;
    let mut first = true;

    for plan in plans {
        let field_val =
            object_get_with_cursor(obj, plan.response_key.as_str(), &mut field_lookup_cursor);

        if !first {
            buffer.put(COMMA);
        }
        first = false;

        buffer.put(QUOTE);
        buffer.put(plan.response_key.as_bytes());
        buffer.put(QUOTE);
        buffer.put(COLON);

        let null_propagation_decision = project_simple_field_value(field_val, plan, buffer);

        if null_propagation_decision.should_propagate() && plan.nullability.is_non_null() {
            return NullPropagationDecision::PropagateNullValue;
        }
    }

    buffer.put(CLOSE_BRACE);
    NullPropagationDecision::KeepNullValue
}

fn project_simple_field_value(
    field_val: Option<&Value>,
    plan: &FieldProjectionPlan,
    buffer: &mut Vec<u8>,
) -> NullPropagationDecision {
    if let Some(field_val) = field_val {
        project_selection_set_simple(field_val, plan, buffer, &plan.nullability)
    } else {
        buffer.put(NULL);
        NullPropagationDecision::PropagateNullValue
    }
}

fn project_selection_set_simple(
    data: &Value,
    selection: &FieldProjectionPlan,
    buffer: &mut Vec<u8>,
    nullability: &FieldNullability,
) -> NullPropagationDecision {
    match data {
        Value::Array(arr) => {
            let null_propagation_checkpoint = buffer.len();
            let list_item_nullability = nullability.list_item();
            let item_non_null = list_item_nullability.is_some_and(FieldNullability::is_non_null);
            buffer.put(OPEN_BRACKET);
            let mut first = true;

            for item in arr.iter() {
                if !first {
                    buffer.put(COMMA);
                }

                let null_propagation_decision = project_selection_set_simple(
                    item,
                    selection,
                    buffer,
                    list_item_nullability.unwrap_or(nullability),
                );

                if null_propagation_decision.should_propagate() && item_non_null {
                    buffer.truncate(null_propagation_checkpoint);
                    buffer.put(NULL);
                    return NullPropagationDecision::PropagateNullValue;
                }

                first = false;
            }

            buffer.put(CLOSE_BRACKET);
            NullPropagationDecision::KeepNullValue
        }
        Value::Object(obj) => match &selection.value {
            ProjectionValueSource::ResponseData {
                selections: Some(selections),
            } => project_selection_set_with_map_simple(obj, selections, buffer),
            ProjectionValueSource::ResponseData { selections: None } => {
                serialize_value_to_buffer(data, buffer);
                NullPropagationDecision::KeepNullValue
            }
            ProjectionValueSource::Null => {
                buffer.put(NULL);
                NullPropagationDecision::PropagateNullValue
            }
        },
        Value::Null => {
            buffer.put(NULL);
            NullPropagationDecision::PropagateNullValue
        }
        _ => {
            serialize_value_to_buffer(data, buffer);
            NullPropagationDecision::KeepNullValue
        }
    }
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
) -> Result<NullPropagationDecision, ProjectionError> {
    let mut field_lookup_cursor = 0;

    for plan in plans {
        if let Some(guard) = &plan.parent_type_guard {
            let name = parent_type_name.get();
            if !guard.matches(name) {
                // Seems like the field projection plan applies to other types, so move to the next one
                continue;
            }
        }

        let field_val =
            object_get_with_cursor(obj, plan.response_key.as_str(), &mut field_lookup_cursor);

        if plan.simple_projection {
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

            let null_propagation_decision = project_simple_field_value(field_val, plan, buffer);

            if null_propagation_decision.should_propagate() && plan.nullability.is_non_null() {
                return Ok(NullPropagationDecision::PropagateNullValue);
            }

            continue;
        }

        let res = if let Some(conditions) = &plan.conditions {
            if plan.conditions_need_type_name {
                let field_type_name_cell = OnceCell::new();
                let field_type_name_fn =
                    || Ok(*field_type_name_cell.get_or_init(|| resolve_type_name(plan, field_val)));
                let parent_type_name_fn = || Ok(parent_type_name.get());
                check(
                    conditions,
                    &parent_type_name_fn,
                    &field_type_name_fn,
                    field_val,
                    variable_values,
                )
            } else {
                check_without_type_names(conditions, field_val, variable_values)
            }
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

                let null_propagation_decision = match &plan.value {
                    ProjectionValueSource::Null => {
                        buffer.put(NULL);
                        NullPropagationDecision::PropagateNullValue
                    }
                    ProjectionValueSource::ResponseData { .. } => {
                        if plan.is_typename {
                            // If the field is TYPENAME_FIELD, we should set it to the parent type name
                            buffer.put(QUOTE);
                            buffer.put(parent_type_name.get().as_bytes());
                            buffer.put(QUOTE);
                            NullPropagationDecision::KeepNullValue
                        } else if let Some(field_val) = field_val {
                            project_selection_set(
                                field_val,
                                errors,
                                plan,
                                variable_values,
                                buffer,
                                parent_type_name.clone(),
                                schema_metadata,
                                &plan.nullability,
                            )?
                        } else {
                            // If the field is not found in the object, set it to Null
                            buffer.put(NULL);
                            NullPropagationDecision::PropagateNullValue
                        }
                    }
                };

                // A `null` value in a non-null position bubbles up
                if null_propagation_decision.should_propagate() && plan.nullability.is_non_null() {
                    return Ok(NullPropagationDecision::PropagateNullValue);
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
                if plan.nullability.is_non_null() {
                    return Ok(NullPropagationDecision::PropagateNullValue);
                }
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
                if plan.nullability.is_non_null() {
                    return Ok(NullPropagationDecision::PropagateNullValue);
                }
            }
        }
    }

    Ok(NullPropagationDecision::KeepNullValue)
}

#[inline]
fn object_get_with_cursor<'a, 'b>(
    obj: &'b [(&'a str, Value<'a>)],
    key: &str,
    cursor: &mut usize,
) -> Option<&'b Value<'a>> {
    let len = obj.len();
    if len == 0 {
        return None;
    }

    let start = (*cursor).min(len);

    for index in start..len {
        if obj[index].0 == key {
            *cursor = index + 1;
            return Some(&obj[index].1);
        }
    }

    for index in 0..start {
        if obj[index].0 == key {
            *cursor = index + 1;
            return Some(&obj[index].1);
        }
    }

    None
}

#[inline]
fn check_without_type_names(
    cond: &FieldProjectionCondition,
    field_value: Option<&Value>,
    variable_values: &Option<HashMap<String, sonic_rs::Value>>,
) -> Result<(), FieldProjectionConditionError> {
    match cond {
        FieldProjectionCondition::And(condition_a, condition_b) => {
            check_without_type_names(condition_a, field_value, variable_values)
                .and_then(|_| check_without_type_names(condition_b, field_value, variable_values))
        }
        FieldProjectionCondition::Or(condition_a, condition_b) => {
            check_without_type_names(condition_a, field_value, variable_values)
                .or_else(|_| check_without_type_names(condition_b, field_value, variable_values))
        }
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
        FieldProjectionCondition::ParentTypeCondition(_)
        | FieldProjectionCondition::FieldTypeCondition(_) => {
            unreachable!("type-name conditions should use check")
        }
    }
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
fn resolve_type_name<'a>(plan: &'a FieldProjectionPlan, field_val: Option<&'a Value>) -> &'a str {
    if plan.is_typename {
        return "String";
    }

    let typename_field = field_val
        .and_then(|value| value.as_object())
        .and_then(|obj| Value::object_get(obj, TYPENAME_FIELD_NAME).and_then(Value::as_str));

    if let Some(typename) = typename_field {
        return typename;
    }

    plan.output_type_name.as_str()
}

#[cfg(test)]
mod tests {
    use graphql_tools::parser::query::Definition;
    use hive_router_query_planner::{
        ast::{document::NormalizedDocument, normalization::create_normalized_document},
        consumer_schema::ConsumerSchema,
        state::supergraph_state::SupergraphState,
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
        let supergraph_state = SupergraphState::new(&supergraph);
        let normalized_operation: NormalizedDocument = create_normalized_document(
            &supergraph_state,
            operation_ast.clone(),
            Some("GetMetadata".into()),
        );
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
            &Default::default(),
            operation_type_name,
            &selections,
            &None,
            1000,
            &schema_metadata,
        );
        let projected_bytes = projection.unwrap();
        let projected_value: sonic_rs::Value = sonic_rs::from_slice(&projected_bytes).unwrap();
        let expected_response: sonic_rs::Value = sonic_rs::from_str(
            r#"{"data":{"metadatas":[{"id":"meta1","data":{"float":41.5,"int":-42,"str":"value1","unsigned":123}}, {"id":"meta2","data":null}]}}"#,
        )
        .unwrap();
        assert_eq!(projected_value, expected_response);
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

        let supergraph_state = SupergraphState::new(&supergraph);
        let normalized_operation: NormalizedDocument = create_normalized_document(
            &supergraph_state,
            operation_ast.clone(),
            Some("SearchQuery".into()),
        );
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
            &Default::default(),
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
