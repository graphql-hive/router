use crate::projection::error::ProjectionError;
use crate::projection::plan::FieldProjectionConditionError;
use crate::response::flat::{FlatResponseData, FlatValue, ObjectId, ValueId};
use crate::response::graphql_error::GraphQLError;
use ahash::AHashMap;
use bytes::BufMut;
use hive_router_query_planner::planner::plan_nodes::{
    CompiledFieldProjectionCondition as FieldProjectionCondition, CompiledFieldProjectionPlan,
    CompiledProjectionValueSource, CompiledTypeCondition as TypeCondition, FieldSymbolId,
    SchemaInterner,
};
use sonic_rs::JsonValueTrait;
use std::cell::OnceCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

#[derive(Default)]
struct ProjectionRuntimeCache;

#[derive(Debug, Clone)]
pub enum StaticProjectionValueSource {
    ResponseData {
        selections: Option<Arc<Vec<StaticFieldProjectionPlan>>>,
    },
    Null,
}

#[derive(Debug, Clone)]
pub struct StaticFieldProjectionPlan {
    pub field_name: String,
    pub response_key: String,
    pub is_typename: bool,
    pub parent_type_guard: Option<TypeCondition>,
    pub conditions: Option<FieldProjectionCondition>,
    pub value: StaticProjectionValueSource,
}

#[derive(Clone)]
enum RuntimeProjectionValueSource {
    ResponseData {
        selections: Option<Arc<Vec<RuntimeFieldProjectionPlan>>>,
    },
    Null,
}

#[derive(Clone)]
struct RuntimeFieldProjectionPlan {
    field_name: String,
    response_key: String,
    field_symbol: Option<FieldSymbolId>,
    is_typename: bool,
    parent_type_guard: Option<TypeCondition>,
    conditions: Option<FieldProjectionCondition>,
    value: RuntimeProjectionValueSource,
}

impl ProjectionRuntimeCache {
    fn value_for_field(
        &mut self,
        data: &FlatResponseData,
        object_id: ObjectId,
        field_symbol: Option<FieldSymbolId>,
    ) -> Option<ValueId> {
        let symbol = field_symbol?;
        data.object_field_in_object_by_symbol(object_id, symbol)
    }
}

fn compile_runtime_projection_plans(
    plans: &[StaticFieldProjectionPlan],
    data: &FlatResponseData,
    cache: &mut AHashMap<(usize, usize, u64), Arc<Vec<RuntimeFieldProjectionPlan>>>,
) -> Arc<Vec<RuntimeFieldProjectionPlan>> {
    let key = (
        plans.as_ptr() as usize,
        plans.len(),
        data.symbol_generation(),
    );
    if let Some(compiled) = cache.get(&key) {
        return Arc::clone(compiled);
    }

    let mut runtime_plans = Vec::with_capacity(plans.len());
    for plan in plans {
        let field_name = plan.field_name.clone();
        let response_key = plan.response_key.clone();
        let field_symbol = data.symbol_for(&response_key);
        let value = match &plan.value {
            StaticProjectionValueSource::ResponseData { selections } => {
                RuntimeProjectionValueSource::ResponseData {
                    selections: selections
                        .as_deref()
                        .map(|s| compile_runtime_projection_plans(s, data, cache)),
                }
            }
            StaticProjectionValueSource::Null => RuntimeProjectionValueSource::Null,
        };

        runtime_plans.push(RuntimeFieldProjectionPlan {
            field_name,
            response_key,
            field_symbol,
            is_typename: plan.is_typename,
            parent_type_guard: plan.parent_type_guard.clone(),
            conditions: plan.conditions.clone(),
            value,
        });
    }

    let runtime_plans = Arc::new(runtime_plans);
    cache.insert(key, Arc::clone(&runtime_plans));
    runtime_plans
}

pub fn compile_static_projection_plans(
    plans: &[CompiledFieldProjectionPlan],
    interner: &SchemaInterner,
) -> Vec<StaticFieldProjectionPlan> {
    plans
        .iter()
        .map(|plan| {
            let value = match &plan.value {
                CompiledProjectionValueSource::ResponseData { selections } => {
                    StaticProjectionValueSource::ResponseData {
                        selections: selections
                            .as_deref()
                            .map(|s| Arc::new(compile_static_projection_plans(s, interner))),
                    }
                }
                CompiledProjectionValueSource::Null => StaticProjectionValueSource::Null,
            };

            StaticFieldProjectionPlan {
                field_name: interner.resolve_field(&plan.field_name).to_string(),
                response_key: interner.resolve_field(&plan.response_key).to_string(),
                is_typename: plan.is_typename,
                parent_type_guard: plan.parent_type_guard.clone(),
                conditions: plan.conditions.clone(),
                value,
            }
        })
        .collect()
}

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
        selection: &'a RuntimeFieldProjectionPlan,
        data_id: Option<ValueId>,
        store: &'a FlatResponseData<'a>,
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
        selection: &'a RuntimeFieldProjectionPlan,
        data_id: Option<ValueId>,
        store: &'a FlatResponseData<'a>,
        parent: TypeName<'a>,
        schema: &'a SchemaMetadata,
    ) -> Self {
        TypeName::Deferred {
            selection,
            data_id,
            store,
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
                data_id,
                store,
                parent,
                schema,
                cached,
            } => cached
                .get_or_init(|| resolve_type_name(selection, *data_id, store, parent, schema))
                .clone(),
        }
    }
}

fn type_condition_matches(condition: &TypeCondition, type_name: &str) -> bool {
    match condition {
        TypeCondition::Exact(expected) => type_name == expected,
        TypeCondition::OneOf(possible) => possible.contains(type_name),
    }
}

// TODO: simplfy args
#[allow(clippy::too_many_arguments)]
pub fn project_by_operation(
    data: &FlatResponseData,
    errors: Vec<GraphQLError>,
    extensions: &HashMap<String, sonic_rs::Value>,
    operation_type_name: &str,
    selections: &[StaticFieldProjectionPlan],
    variable_values: &Option<HashMap<String, sonic_rs::Value>>,
    response_size_estimate: usize,
    schema_metadata: &SchemaMetadata,
) -> Result<Vec<u8>, ProjectionError> {
    let mut runtime_cache = ProjectionRuntimeCache::default();
    let mut runtime_plan_cache = AHashMap::new();
    let runtime_plans = compile_runtime_projection_plans(selections, data, &mut runtime_plan_cache);
    let mut buffer = Vec::with_capacity(response_size_estimate);
    buffer.put(OPEN_BRACE);
    buffer.put(QUOTE);
    buffer.put("data".as_bytes());
    buffer.put(QUOTE);
    buffer.put(COLON);

    let mut errors = errors;

    if let Some(root_object_id) = data.root_object_id() {
        // Start with first as true to add the opening brace
        let mut first = true;
        project_selection_set_with_map(
            data,
            root_object_id,
            &mut runtime_cache,
            &mut errors,
            runtime_plans.as_slice(),
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

fn serialize_flat_value_to_buffer(data: &FlatResponseData, id: ValueId, buffer: &mut Vec<u8>) {
    match data.value_kind(id) {
        Some(FlatValue::Null) | None => buffer.put(NULL),
        Some(FlatValue::Bool(true)) => buffer.put(TRUE),
        Some(FlatValue::Bool(false)) => buffer.put(FALSE),
        Some(FlatValue::U64(num)) => write_u64(buffer, *num),
        Some(FlatValue::I64(num)) => write_i64(buffer, *num),
        Some(FlatValue::F64(num)) => write_f64(buffer, *num),
        Some(FlatValue::String(value)) => write_and_escape_string(buffer, value),
        Some(FlatValue::Object(obj_id)) => {
            buffer.put(OPEN_BRACE);
            let mut first = true;
            if let Some(fields) = data.object_fields(*obj_id) {
                for (key, value_id) in fields {
                    if !first {
                        buffer.put(COMMA);
                    }
                    write_and_escape_string(buffer, key.as_ref());
                    buffer.put(COLON);
                    serialize_flat_value_to_buffer(data, *value_id, buffer);
                    first = false;
                }
            }
            buffer.put(CLOSE_BRACE);
        }
        Some(FlatValue::List(list_id)) => {
            buffer.put(OPEN_BRACKET);
            let mut first = true;
            if let Some(items) = data.list_items(*list_id) {
                for item in items {
                    if !first {
                        buffer.put(COMMA);
                    }
                    serialize_flat_value_to_buffer(data, *item, buffer);
                    first = false;
                }
            }
            buffer.put(CLOSE_BRACKET);
        }
    }
}

fn project_selection_set<'a>(
    data: &'a FlatResponseData<'a>,
    value_id: ValueId,
    runtime_cache: &mut ProjectionRuntimeCache,
    errors: &mut Vec<GraphQLError>,
    selection: &'a RuntimeFieldProjectionPlan,
    variable_values: &Option<HashMap<String, sonic_rs::Value>>,
    buffer: &mut Vec<u8>,
    parent_type_name: TypeName<'a>,
    schema_metadata: &'a SchemaMetadata,
) -> Result<(), ProjectionError> {
    match data.value_kind(value_id) {
        Some(FlatValue::List(list_id)) => {
            buffer.put(OPEN_BRACKET);
            let mut first = true;
            for item in data.list_items(*list_id).unwrap_or(&[]) {
                if !first {
                    buffer.put(COMMA);
                }
                project_selection_set(
                    data,
                    *item,
                    runtime_cache,
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
        Some(FlatValue::Object(object_id)) => {
            match &selection.value {
                RuntimeProjectionValueSource::ResponseData {
                    selections: Some(selections),
                } => {
                    let mut first = true;
                    let type_name = TypeName::deferred(
                        selection,
                        Some(value_id),
                        data,
                        parent_type_name,
                        schema_metadata,
                    );
                    project_selection_set_with_map(
                        data,
                        *object_id,
                        runtime_cache,
                        errors,
                        selections.as_slice(),
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
                RuntimeProjectionValueSource::ResponseData { selections: None } => {
                    // If the selection has no sub-selections, we serialize the whole object
                    serialize_flat_value_to_buffer(data, value_id, buffer);
                }
                RuntimeProjectionValueSource::Null => {
                    // This should not happen as we are in an object case, but just in case
                    buffer.put(NULL);
                }
            }
        }
        _ => {
            // If the data is not an object or array, we serialize it directly
            serialize_flat_value_to_buffer(data, value_id, buffer);
        }
    };
    Ok(())
}

// TODO: simplfy args
#[allow(clippy::too_many_arguments)]
fn project_selection_set_with_map<'a>(
    data: &'a FlatResponseData<'a>,
    object_id: ObjectId,
    runtime_cache: &mut ProjectionRuntimeCache,
    errors: &mut Vec<GraphQLError>,
    plans: &'a [RuntimeFieldProjectionPlan],
    variable_values: &Option<HashMap<String, sonic_rs::Value>>,
    parent_type_name: TypeName<'a>,
    buffer: &mut Vec<u8>,
    first: &mut bool,
    schema_metadata: &'a SchemaMetadata,
) -> Result<(), ProjectionError> {
    for plan in plans {
        if let Some(guard) = &plan.parent_type_guard {
            let name = parent_type_name.get()?;
            if !type_condition_matches(guard, name) {
                // Seems like the field projection plan applies to other types, so move to the next one
                continue;
            }
        }

        let field_val = runtime_cache.value_for_field(data, object_id, plan.field_symbol);

        let res = if let Some(conditions) = &plan.conditions {
            let field_type_name_cell = OnceCell::new();
            let field_type_name_fn = || {
                field_type_name_cell
                    .get_or_init(|| {
                        resolve_type_name(plan, field_val, data, &parent_type_name, schema_metadata)
                    })
                    .clone()
            };
            let parent_type_name_fn = || parent_type_name.get();
            check(
                conditions,
                &parent_type_name_fn,
                &field_type_name_fn,
                field_val,
                data,
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
                    RuntimeProjectionValueSource::Null => {
                        buffer.put(NULL);
                        continue;
                    }
                    RuntimeProjectionValueSource::ResponseData { selections: None } => {
                        if plan.is_typename {
                            buffer.put(QUOTE);
                            buffer.put(parent_type_name.get()?.as_bytes());
                            buffer.put(QUOTE);
                        } else if let Some(field_val) = field_val {
                            serialize_flat_value_to_buffer(data, field_val, buffer);
                        } else {
                            buffer.put(NULL);
                        }
                    }
                    RuntimeProjectionValueSource::ResponseData { .. } => {
                        if plan.is_typename {
                            // If the field is TYPENAME_FIELD, we should set it to the parent type name
                            buffer.put(QUOTE);
                            buffer.put(parent_type_name.get()?.as_bytes());
                            buffer.put(QUOTE);
                        } else if let Some(field_val) = field_val {
                            project_selection_set(
                                data,
                                field_val,
                                runtime_cache,
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
    field_value: Option<ValueId>,
    data: &'a FlatResponseData<'a>,
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
            data,
            variable_values,
        )
        .and_then(|_| {
            check(
                condition_b,
                parent_type_name,
                field_type_name,
                field_value,
                data,
                variable_values,
            )
        }),
        FieldProjectionCondition::Or(condition_a, condition_b) => check(
            condition_a,
            parent_type_name,
            field_type_name,
            field_value,
            data,
            variable_values,
        )
        .or_else(|_| {
            check(
                condition_b,
                parent_type_name,
                field_type_name,
                field_value,
                data,
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
            if type_condition_matches(type_condition, parent_type_name()?) {
                Ok(())
            } else {
                Err(FieldProjectionConditionError::InvalidParentType)
            }
        }
        FieldProjectionCondition::FieldTypeCondition(type_condition) => {
            if type_condition_matches(type_condition, field_type_name()?) {
                Ok(())
            } else {
                Err(FieldProjectionConditionError::InvalidFieldType)
            }
        }
        FieldProjectionCondition::EnumValuesCondition(enum_values) => {
            if let Some(string_value) = field_value.and_then(|id| data.value_as_str(id)) {
                if enum_values.contains(string_value) {
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
    plan: &'a RuntimeFieldProjectionPlan,
    field_val: Option<ValueId>,
    data: &'a FlatResponseData<'a>,
    parent_type_name: &TypeName<'a>,
    schema_metadata: &'a SchemaMetadata,
) -> Result<&'a str, ProjectionError> {
    if plan.is_typename {
        return Ok("String");
    }

    let typename_field = field_val
        .and_then(|id| data.object_field(id, TYPENAME_FIELD_NAME))
        .and_then(|id| data.value_as_str(id));

    if let Some(typename) = typename_field {
        return Ok(typename);
    }

    let parent_type_name = parent_type_name.get()?;

    let fields = schema_metadata
        .get_type_fields(parent_type_name)
        .ok_or_else(|| ProjectionError::MissingType(parent_type_name.to_string()))?;

    fields
        .get(plan.field_name.as_str())
        .map(|field_info| field_info.output_type_name.as_str())
        .ok_or_else(|| ProjectionError::MissingField {
            field_name: plan.field_name.clone(),
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
        projection::{plan::compile_projection_from_operation, response::project_by_operation},
        response::flat::FlatResponseData,
    };
    use hive_router_query_planner::planner::plan_nodes::SchemaInterner;

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
        let interner = SchemaInterner::default();
        let (operation_type_name, selections) = compile_projection_from_operation(
            &normalized_operation.operation,
            &schema_metadata,
            &interner,
        );
        let static_selections = super::compile_static_projection_plans(&selections, &interner);
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
        let data = FlatResponseData::from_sonic_value_ref(data_json.as_ref());
        let projection = project_by_operation(
            &data,
            vec![],
            &HashMap::new(),
            operation_type_name,
            &static_selections,
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
            create_normalized_document(operation_ast.clone(), Some("SearchQuery"));
        let interner = SchemaInterner::default();
        let (operation_type_name, selections) = compile_projection_from_operation(
            &normalized_operation.operation,
            &schema_metadata,
            &interner,
        );
        let static_selections = super::compile_static_projection_plans(&selections, &interner);

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
        let data = FlatResponseData::from_sonic_value_ref(data_json.as_ref());
        let projection = project_by_operation(
            &data,
            vec![],
            &HashMap::new(),
            operation_type_name,
            &static_selections,
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
