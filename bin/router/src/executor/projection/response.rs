use crate::executor::execution::plan::ExecutionResultExtensions;
use crate::executor::projection::error::ProjectionError;
use crate::executor::projection::plan::{
    Condition, ConditionId, FieldRecord, ProjectionPlan, ShapeFlags,
};
use crate::executor::response::graphql_error::GraphQLError;
use crate::executor::response::value::Value;
use bytes::BufMut;
use sonic_rs::JsonValueTrait;
use std::cell::OnceCell;
use std::collections::HashMap;

use crate::executor::introspection::schema::SchemaMetadata;
use crate::executor::json_writer::{write_and_escape_string, write_f64, write_i64, write_u64};
use crate::executor::utils::consts::{
    CLOSE_BRACE, CLOSE_BRACKET, COLON, COMMA, EMPTY_OBJECT, FALSE, NULL, OPEN_BRACE, OPEN_BRACKET,
    QUOTE, TRUE, TYPENAME_FIELD_NAME,
};

enum NullPropagationDecision {
    /// The value is `null` and may need to bubble up.
    PropagateNullValue,
    /// The value can stay as-is.
    KeepNullValue,
}

impl NullPropagationDecision {
    #[inline]
    fn should_propagate(&self) -> bool {
        matches!(self, NullPropagationDecision::PropagateNullValue)
    }
}

#[derive(Debug)]
enum ConditionFailure {
    InvalidParentType,
    InvalidFieldType,
    Skip,
    InvalidEnumValue,
    Fatal(ProjectionError),
}

impl From<ProjectionError> for ConditionFailure {
    fn from(err: ProjectionError) -> Self {
        ConditionFailure::Fatal(err)
    }
}

#[derive(Clone, Copy)]
struct ShapeCursor<'a>(&'a [u8]);

impl<'a> ShapeCursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self(bytes)
    }

    fn is_non_null(self) -> bool {
        self.0.first().is_some_and(|marker| {
            ShapeFlags::from_bits_retain(*marker).contains(ShapeFlags::NON_NULL)
        })
    }

    fn list_item(self) -> Option<Self> {
        match self.0.split_first() {
            Some((marker, rest))
                if ShapeFlags::from_bits_retain(*marker).contains(ShapeFlags::LIST) =>
            {
                Some(Self(rest))
            }
            _ => None,
        }
    }
}

/// A type name that is either known or resolved when needed.
enum TypeName<'a, 'ctx> {
    Resolved(&'a str),
    Deferred {
        selection: &'a FieldRecord,
        plan: &'a ProjectionPlan,
        data: Option<&'a Value<'a>>,
        parent: &'ctx TypeName<'a, 'ctx>,
        schema: &'a SchemaMetadata,
        /// Cache the resolved name.
        cached: OnceCell<Result<&'a str, ProjectionError>>,
    },
}

impl<'a, 'ctx> TypeName<'a, 'ctx> {
    #[inline]
    fn resolved(type_name: &'a str) -> Self {
        TypeName::Resolved(type_name)
    }

    #[inline]
    fn deferred(
        selection: &'a FieldRecord,
        plan: &'a ProjectionPlan,
        data: Option<&'a Value>,
        parent: &'ctx TypeName<'a, 'ctx>,
        schema: &'a SchemaMetadata,
    ) -> Self {
        TypeName::Deferred {
            selection,
            plan,
            data,
            parent,
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
                plan,
                data,
                parent,
                schema,
                cached,
            } => cached
                .get_or_init(|| resolve_type_name(selection, *data, parent, plan, schema))
                .clone(),
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn project_by_operation(
    data: &Value,
    errors: Vec<GraphQLError>,
    extensions: &ExecutionResultExtensions<'_>,
    operation_type_name: &str,
    plan: &ProjectionPlan,
    variable_values: &Option<HashMap<String, sonic_rs::Value>>,
    response_size_estimate: usize,
    schema_metadata: &SchemaMetadata,
) -> Result<Vec<u8>, ProjectionError> {
    let mut out = Projector {
        plan,
        schema: schema_metadata,
        variables: variable_values,
        errors,
        buffer: Vec::with_capacity(response_size_estimate),
    };
    out.buffer.put(OPEN_BRACE);
    out.buffer.put(QUOTE);
    out.buffer.put("data".as_bytes());
    out.buffer.put(QUOTE);
    out.buffer.put(COLON);

    if let Some(data_map) = data.as_object() {
        let null_propagation_checkpoint = out.buffer.len();
        // Start with first as true to add the opening brace
        let mut first = true;
        let root_fields = plan.root_fields();
        let null_propagation_decision = out.project_object_fields(
            data_map,
            root_fields,
            &TypeName::resolved(operation_type_name),
            &mut first,
            &mut [],
        )?;

        if null_propagation_decision.should_propagate() {
            out.buffer.truncate(null_propagation_checkpoint);
            out.buffer.put(NULL);
        } else if !first {
            out.buffer.put(CLOSE_BRACE);
        } else {
            // If no selections were made, we should return an empty object
            out.buffer.put(EMPTY_OBJECT);
        }
    } else {
        out.buffer.put(NULL);
    }

    if !out.errors.is_empty() {
        let serialized = sonic_rs::to_vec(&out.errors)
            .map_err(|e| ProjectionError::ErrorsSerializationFailure(e.to_string()))?;
        out.buffer.put(COMMA);
        out.buffer.put(QUOTE);
        out.buffer.put("errors".as_bytes());
        out.buffer.put(QUOTE);
        out.buffer.put(COLON);
        out.buffer.put_slice(&serialized);
    }

    if !extensions.is_empty() {
        let serialized_extensions = sonic_rs::to_vec(extensions)
            .map_err(|e| ProjectionError::ExtensionsSerializationFailure(e.to_string()))?;
        out.buffer.put(COMMA);
        out.buffer.put(QUOTE);
        out.buffer.put("extensions".as_bytes());
        out.buffer.put(QUOTE);
        out.buffer.put(COLON);
        out.buffer.put_slice(&serialized_extensions);
    }

    out.buffer.put(CLOSE_BRACE);
    Ok(out.buffer)
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

/// Cached field positions kept on the stack before falling back to the heap.
const STACK_CACHE_FIELD_LIMIT: usize = 16;
/// Marks a field that was not found.
const MISSING_FIELD_INDEX: usize = usize::MAX;

struct Projector<'a, 'v> {
    plan: &'a ProjectionPlan,
    schema: &'a SchemaMetadata,
    variables: &'v Option<HashMap<String, sonic_rs::Value>>,
    errors: Vec<GraphQLError>,
    buffer: Vec<u8>,
}

impl<'a> Projector<'a, '_> {
    fn project_value(
        &mut self,
        data: &'a Value<'a>,
        selection: &'a FieldRecord,
        parent_type_name: &TypeName<'a, '_>,
        nullability: Option<ShapeCursor<'_>>,
        indexes: &mut [usize],
    ) -> Result<NullPropagationDecision, ProjectionError> {
        match data {
            Value::Array(arr) => {
                // Reuse field positions across objects in the same list.
                let cache_size = if indexes.is_empty() && arr.len() > 1 && selection.has_children()
                {
                    Some(selection.children.len as usize)
                } else {
                    None
                };
                let mut heap_cache = match cache_size {
                    Some(len) if len > STACK_CACHE_FIELD_LIMIT => vec![MISSING_FIELD_INDEX; len],
                    _ => Vec::new(),
                };
                let mut stack_cache = [MISSING_FIELD_INDEX; STACK_CACHE_FIELD_LIMIT];
                let indexes = match cache_size {
                    Some(len) if len <= STACK_CACHE_FIELD_LIMIT => &mut stack_cache[..len],
                    Some(_) => heap_cache.as_mut_slice(),
                    _ => indexes,
                };
                let null_propagation_checkpoint = self.buffer.len();
                let item_shape = nullability.and_then(ShapeCursor::list_item);
                let item_non_null = item_shape.is_some_and(ShapeCursor::is_non_null);
                self.buffer.put(OPEN_BRACKET);
                let mut first = true;
                for item in arr.iter() {
                    if !first {
                        self.buffer.put(COMMA);
                    }
                    let needs_null_propagation = self.project_value(
                        item,
                        selection,
                        parent_type_name,
                        item_shape.or(nullability),
                        indexes,
                    )?;

                    if needs_null_propagation.should_propagate() && item_non_null {
                        self.buffer.truncate(null_propagation_checkpoint);
                        self.buffer.put(NULL);
                        return Ok(NullPropagationDecision::PropagateNullValue);
                    }

                    first = false;
                }

                self.buffer.put(CLOSE_BRACKET);
                Ok(NullPropagationDecision::KeepNullValue)
            }
            Value::Object(obj) if selection.has_children() => {
                let null_propagation_checkpoint = self.buffer.len();
                let mut first = true;
                let type_name = TypeName::deferred(
                    selection,
                    self.plan,
                    Some(data),
                    parent_type_name,
                    self.schema,
                );
                let fields = self.plan.fields(selection.children);
                let null_propagation_decision =
                    self.project_object_fields(obj, fields, &type_name, &mut first, indexes)?;

                if null_propagation_decision.should_propagate() {
                    self.buffer.truncate(null_propagation_checkpoint);
                    self.buffer.put(NULL);
                    return Ok(NullPropagationDecision::PropagateNullValue);
                }

                if !first {
                    self.buffer.put(CLOSE_BRACE);
                } else {
                    self.buffer.put(EMPTY_OBJECT);
                }
                Ok(NullPropagationDecision::KeepNullValue)
            }
            Value::Null => {
                self.buffer.put(NULL);
                Ok(NullPropagationDecision::PropagateNullValue)
            }
            _ => {
                serialize_value_to_buffer(data, &mut self.buffer);
                Ok(NullPropagationDecision::KeepNullValue)
            }
        }
    }

    fn project_object_fields(
        &mut self,
        obj: &'a [(&str, Value<'a>)],
        fields: &'a [FieldRecord],
        parent_type_name: &TypeName<'a, '_>,
        first: &mut bool,
        indexes: &mut [usize],
    ) -> Result<NullPropagationDecision, ProjectionError> {
        for (offset, field) in fields.iter().enumerate() {
            let response_key = self.plan.response_key(field);
            if let Some(guard) = field.parent_guard {
                if !self.plan.guard_matches(guard, parent_type_name.get()?) {
                    continue;
                }
            }

            let field_val = find_field(obj, response_key, indexes.get_mut(offset));

            let res = if let Some(condition) = field.condition {
                let field_type_name_cell = OnceCell::new();
                let field_type_name_fn = || {
                    field_type_name_cell
                        .get_or_init(|| {
                            resolve_type_name(
                                field,
                                field_val,
                                parent_type_name,
                                self.plan,
                                self.schema,
                            )
                        })
                        .clone()
                };
                let parent_type_name_fn = || parent_type_name.get();
                evaluate(
                    condition,
                    self.plan,
                    &parent_type_name_fn,
                    &field_type_name_fn,
                    field_val,
                    self.variables,
                )
            } else {
                Ok(())
            };

            match res {
                Ok(()) => {
                    let non_null = field.is_non_null();
                    write_key(&mut self.buffer, first, response_key);

                    let null_propagation_decision = if field.is_null_value() {
                        self.buffer.put(NULL);
                        NullPropagationDecision::PropagateNullValue
                    } else if field.is_typename() {
                        self.buffer.put(QUOTE);
                        self.buffer.put(parent_type_name.get()?.as_bytes());
                        self.buffer.put(QUOTE);
                        NullPropagationDecision::KeepNullValue
                    } else if let Some(field_val) = field_val {
                        let nullability = matches!(field_val, Value::Array(_))
                            .then(|| ShapeCursor::new(self.plan.shape(field.nullability())));
                        self.project_value(
                            field_val,
                            field,
                            parent_type_name,
                            nullability,
                            &mut [],
                        )?
                    } else {
                        self.buffer.put(NULL);
                        NullPropagationDecision::PropagateNullValue
                    };

                    // A `null` value in a non-null position bubbles up
                    if null_propagation_decision.should_propagate() && non_null {
                        return Ok(NullPropagationDecision::PropagateNullValue);
                    }
                }
                Err(ConditionFailure::Fatal(error)) => {
                    return Err(error);
                }
                Err(ConditionFailure::Skip | ConditionFailure::InvalidParentType) => continue,
                Err(
                    failure @ (ConditionFailure::InvalidEnumValue
                    | ConditionFailure::InvalidFieldType),
                ) => {
                    let non_null = field.is_non_null();
                    write_key(&mut self.buffer, first, response_key);
                    self.buffer.put(NULL);
                    if matches!(failure, ConditionFailure::InvalidEnumValue) {
                        self.errors
                            .push(GraphQLError::from("Value is not a valid enum value"));
                    }
                    if non_null {
                        return Ok(NullPropagationDecision::PropagateNullValue);
                    }
                }
            }
        }
        Ok(NullPropagationDecision::KeepNullValue)
    }
}

/// Finds `__typename` in a sorted object.
#[inline]
fn find_typename<'a>(object: &'a [(&str, Value)]) -> Option<&'a str> {
    if let Some((key, value)) = object.first() {
        if *key == TYPENAME_FIELD_NAME {
            return value.as_str();
        }
    }
    object
        .binary_search_by_key(&TYPENAME_FIELD_NAME, |(key, _)| *key)
        .ok()
        .and_then(|index| object[index].1.as_str())
}

#[inline]
fn find_field<'a>(
    obj: &'a [(&str, Value)],
    response_key: &str,
    hint: Option<&mut usize>,
) -> Option<&'a Value<'a>> {
    // The saved position is only a hint: check that the key still matches.
    if let Some((key, value)) = hint.as_ref().and_then(|hint| obj.get(**hint)) {
        if *key == response_key {
            return Some(value);
        }
    }

    // A previous object may have left the field out or kept it in a different
    // spot, so always search the current object and save what is found.
    let found = obj
        .binary_search_by_key(&response_key, |(key, _)| *key)
        .ok();
    if let Some(hint) = hint {
        *hint = found.unwrap_or(MISSING_FIELD_INDEX);
    }
    found.map(|index| &obj[index].1)
}

#[inline(always)]
fn write_key(buffer: &mut Vec<u8>, first: &mut bool, key: &str) {
    if *first {
        buffer.put(OPEN_BRACE);
    } else {
        buffer.put(COMMA);
    }
    *first = false;
    buffer.put(QUOTE);
    // GraphQL field names and aliases are valid JSON-safe name tokens.
    buffer.put(key.as_bytes());
    buffer.put(QUOTE);
    buffer.put(COLON);
}

#[inline]
/// Resolves a field type from response data or schema metadata.
fn resolve_type_name<'a>(
    field: &'a FieldRecord,
    field_value: Option<&'a Value>,
    parent_type_name: &TypeName<'a, '_>,
    plan: &'a ProjectionPlan,
    schema_metadata: &'a SchemaMetadata,
) -> Result<&'a str, ProjectionError> {
    if field.is_typename() {
        return Ok("String");
    }
    if let Some(typename) = field_value
        .and_then(|value| value.as_object())
        .and_then(|object| find_typename(object))
    {
        return Ok(typename);
    }
    let parent = parent_type_name.get()?;
    let fields = schema_metadata
        .get_type_fields(parent)
        .ok_or_else(|| ProjectionError::MissingType(parent.to_string()))?;
    fields
        .get(plan.field_name(field))
        .map(|field| field.output_type_name.as_str())
        .ok_or_else(|| ProjectionError::MissingField {
            field_name: plan.field_name(field).to_string(),
            type_name: parent.to_string(),
        })
}

#[inline]
fn evaluate<'a, F, T>(
    id: ConditionId,
    plan: &ProjectionPlan,
    parent_type_name: &T,
    field_type_name: &F,
    field_value: Option<&Value>,
    variable_values: &Option<HashMap<String, sonic_rs::Value>>,
) -> Result<(), ConditionFailure>
where
    F: Fn() -> Result<&'a str, ProjectionError>,
    T: Fn() -> Result<&'a str, ProjectionError>,
{
    let condition = plan.condition(id);
    match condition {
        Condition::Include(variable) => variable_values
            .as_ref()
            .and_then(|values| values.get(plan.text(variable)))
            .and_then(|value| value.as_bool())
            .filter(|value| *value)
            .map(|_| ())
            .ok_or(ConditionFailure::Skip),
        Condition::Skip(variable) => {
            if variable_values
                .as_ref()
                .and_then(|values| values.get(plan.text(variable)))
                .and_then(|value| value.as_bool())
                .unwrap_or(false)
            {
                Err(ConditionFailure::Skip)
            } else {
                Ok(())
            }
        }
        Condition::ParentType(guard) => {
            if plan.guard_matches(guard, parent_type_name()?) {
                Ok(())
            } else {
                Err(ConditionFailure::InvalidParentType)
            }
        }
        Condition::FieldType(guard) => {
            if plan.guard_matches(guard, field_type_name()?) {
                Ok(())
            } else {
                Err(ConditionFailure::InvalidFieldType)
            }
        }
        Condition::EnumValues(set) => {
            if let Some(Value::String(value)) = field_value {
                if plan.set_matches(set, value) {
                    Ok(())
                } else {
                    Err(ConditionFailure::InvalidEnumValue)
                }
            } else {
                Ok(())
            }
        }
        Condition::And(left, right) => evaluate(
            left,
            plan,
            parent_type_name,
            field_type_name,
            field_value,
            variable_values,
        )
        .and_then(|_| {
            evaluate(
                right,
                plan,
                parent_type_name,
                field_type_name,
                field_value,
                variable_values,
            )
        }),
        Condition::Or(left, right) => evaluate(
            left,
            plan,
            parent_type_name,
            field_type_name,
            field_value,
            variable_values,
        )
        .or_else(|_| {
            evaluate(
                right,
                plan,
                parent_type_name,
                field_type_name,
                field_value,
                variable_values,
            )
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::introspection::schema::SchemaMetadata;
    use crate::executor::introspection::schema::SchemaWithMetadata;
    use crate::query_planner::ast::normalization::normalize_operation;
    use crate::query_planner::consumer_schema::ConsumerSchema;
    use crate::query_planner::utils::parsing::{parse_operation, parse_schema};
    use crate::query_planner::{planner::Planner, state::supergraph_state::SupergraphState};

    #[derive(Clone, Copy)]
    struct ProjectionCase {
        name: &'static str,
        field: &'static str,
        input: &'static str,
        expected: &'static str,
    }

    const LIST: u8 = 1;
    const NON_NULL: u8 = 2;
    const NON_NULL_LIST: u8 = 3;
    const NULLABLE: u8 = 0;

    fn project(
        supergraph_state: &SupergraphState,
        schema_metadata: &SchemaMetadata,
        operation: &str,
        data: &str,
        variables: Option<HashMap<String, sonic_rs::Value>>,
    ) -> String {
        let operation = parse_operation(operation);
        let normalized = normalize_operation(supergraph_state, &operation, None).unwrap();
        let (root_type_name, plan) =
            ProjectionPlan::from_operation(normalized.executable_operation(), schema_metadata);
        let data_json: sonic_rs::Value = sonic_rs::from_str(data).unwrap();
        let data = Value::from(data_json.as_ref());

        project_plan(&data, root_type_name, &plan, variables, schema_metadata)
    }

    fn project_plan(
        data: &Value,
        root_type_name: &str,
        plan: &ProjectionPlan,
        variables: Option<HashMap<String, sonic_rs::Value>>,
        schema_metadata: &SchemaMetadata,
    ) -> String {
        let output = project_by_operation(
            data,
            vec![],
            &Default::default(),
            root_type_name,
            plan,
            &variables,
            1024,
            schema_metadata,
        )
        .unwrap();
        String::from_utf8(output).unwrap()
    }

    #[test]
    fn project_scalars_with_object_value() {
        let supergraph = parse_schema(
            r#"
            type Query { metadatas: [Metadata!]! }
            scalar JSON
            type Metadata { id: ID!, timestamp: String!, data: JSON }
            "#,
        );
        let consumer_schema = ConsumerSchema::new_from_supergraph(&supergraph);
        let schema_metadata = consumer_schema.schema_metadata();
        let supergraph_state = SupergraphState::new(&supergraph);
        let operation = parse_operation(
            r#"
            query GetMetadata { metadatas { id data } }
            "#,
        );
        let normalized = normalize_operation(&supergraph_state, &operation, None).unwrap();
        let (root_type_name, plan) =
            ProjectionPlan::from_operation(normalized.executable_operation(), &schema_metadata);

        let data_json = sonic_rs::json!({
            "__typename": "Query",
            "metadatas": [
                {
                    "__typename": "Metadata",
                    "id": "meta1",
                    "timestamp": "2024-01-01T00:00:00Z",
                    "data": { "float": 41.5, "int": -42, "str": "value1", "unsigned": 123 }
                },
                { "__typename": "Metadata", "id": "meta2", "data": null }
            ]
        });
        let data = Value::from(data_json.as_ref());
        let output = project_by_operation(
            &data,
            vec![],
            &Default::default(),
            root_type_name,
            &plan,
            &None,
            1000,
            &schema_metadata,
        )
        .unwrap();

        assert_eq!(
            String::from_utf8(output).unwrap(),
            r#"{"data":{"metadatas":[{"id":"meta1","data":{"float":41.5,"int":-42,"str":"value1","unsigned":123}},{"id":"meta2","data":null}]}}"#
        );
    }

    /// Two fragments select the same response key on different concrete types,
    /// with different child types. Merging has to keep them apart, or the same
    /// key gets written twice and the JSON comes out malformed.
    #[test]
    fn duplicate_selections_in_merged_plans() {
        let supergraph = parse_schema(
            r#"
            interface Node { id: ID! }
            type A implements Node { id: ID, children: [AChild] }
            type B implements Node { id: ID!, children: [BChild] }
            type AChild { id: ID }
            type BChild { id: ID }
            type Container { node: Node }
            type Query { nodes: [Container] }
            "#,
        );
        let consumer_schema = ConsumerSchema::new_from_supergraph(&supergraph);
        let schema_metadata = consumer_schema.schema_metadata();
        let supergraph_state = SupergraphState::new(&supergraph);
        let operation = parse_operation(
            r#"
            query {
              nodes {
                node {
                  ... on A { children { id } }
                  ... on B { children { id } }
                }
              }
            }
            "#,
        );
        let normalized = normalize_operation(&supergraph_state, &operation, None).unwrap();
        let (root_type_name, plan) =
            ProjectionPlan::from_operation(normalized.executable_operation(), &schema_metadata);

        // One empty list and one populated, so the list index cache is reused
        // across items that do not agree on which fields are present.
        let data_json = sonic_rs::json!({
            "__typename": "Query",
            "nodes": [
                { "node": { "__typename": "A", "children": [] } },
                { "node": { "__typename": "B", "children": [{ "id": "b_child_1" }] } }
            ]
        });
        let data = Value::from(data_json.as_ref());
        let output = project_by_operation(
            &data,
            vec![],
            &Default::default(),
            root_type_name,
            &plan,
            &None,
            1000,
            &schema_metadata,
        )
        .unwrap();

        // Compared as raw bytes, which is what the client actually gets, and
        // what the other tests here do.
        assert_eq!(
            String::from_utf8(output).unwrap(),
            r#"{"data":{"nodes":[{"node":{"children":[]}},{"node":{"children":[{"id":"b_child_1"}]}}]}}"#
        );
    }

    #[test]
    fn unconditional_overlap_survives_false_directive_at_execution() {
        let schema = parse_schema(
            r#"
                interface Node { id: ID! }
                type User implements Node { id: ID!, name: String }
                type Query { node: Node }
                "#,
        );
        let supergraph = SupergraphState::new(&schema);
        let planner = Planner::new_from_supergraph(&schema, Default::default()).unwrap();
        let operation = parse_operation(
            r#"
                query Example($show: Boolean!) {
                  node {
                    ... on User { label: name @include(if: $show) }
                    ... on User { label: name }
                  }
                }
                "#,
        );
        let normalized = normalize_operation(&supergraph, &operation, None).unwrap();
        let (_, plan) = ProjectionPlan::from_operation(
            normalized.executable_operation(),
            &planner.consumer_schema.schema_metadata(),
        );
        let data = Value::Object(vec![(
            "node",
            Value::Object(vec![
                ("__typename", Value::String("User".into())),
                ("label", Value::String("Ada".into())),
            ]),
        )]);
        let mut variables = HashMap::new();
        variables.insert("show".into(), sonic_rs::Value::from(false));
        let output = project_by_operation(
            &data,
            vec![],
            &Default::default(),
            "Query",
            &plan,
            &Some(variables),
            128,
            &planner.consumer_schema.schema_metadata(),
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(output).unwrap(),
            r#"{"data":{"node":{"label":"Ada"}}}"#
        );
    }

    /// Null-propagation matrix for nested object lists through `ShapeCursor`.
    #[test]
    fn nested_list_nullability_matrix() {
        let supergraph = parse_schema(
            r#"
            type Query {
                strict: [[Foo!]!]!
                nullable: [[Foo]]
                boundary: [[Foo!]!]
                mixed: [Foo!]!
            }
            type Foo { name: String!, nick: String }
            "#,
        );
        let consumer_schema = ConsumerSchema::new_from_supergraph(&supergraph);
        let schema_metadata = consumer_schema.schema_metadata();
        let supergraph_state = SupergraphState::new(&supergraph);

        // Shape bytes are one flag byte per level, outside in.
        for (field, expected_shape) in [
            ("strict", &[NON_NULL_LIST, NON_NULL_LIST, NON_NULL][..]),
            ("nullable", &[LIST, LIST, NULLABLE][..]),
            ("boundary", &[LIST, NON_NULL_LIST, NON_NULL][..]),
            ("mixed", &[NON_NULL_LIST, NON_NULL][..]),
        ] {
            let operation = parse_operation(&format!(r#"query {{ {field} {{ name nick }} }}"#));
            let normalized = normalize_operation(&supergraph_state, &operation, None).unwrap();
            let (_, plan) =
                ProjectionPlan::from_operation(normalized.executable_operation(), &schema_metadata);
            let root_fields = plan.root_fields();
            assert_eq!(root_fields.len(), 1, "one root field for {field}");
            assert_eq!(
                plan.shape(root_fields[0].nullability()),
                expected_shape,
                "nullability shape for {field}"
            );
        }

        let cases = [
            ProjectionCase {
                name: "strict / valid",
                field: "strict",
                input: r#"[[{"__typename":"Foo","name":"a","nick":"b"}]]"#,
                expected: r#"{"data":{"strict":[[{"name":"a","nick":"b"}]]}}"#,
            },
            ProjectionCase {
                name: "strict / nullable leaf",
                field: "strict",
                input: r#"[[{"__typename":"Foo","name":"a","nick":null}]]"#,
                expected: r#"{"data":{"strict":[[{"name":"a","nick":null}]]}}"#,
            },
            ProjectionCase {
                name: "strict / non-null leaf",
                field: "strict",
                input: r#"[[{"__typename":"Foo","name":null,"nick":"b"}]]"#,
                expected: r#"{"data":null}"#,
            },
            ProjectionCase {
                name: "strict / null object",
                field: "strict",
                input: r#"[[null]]"#,
                expected: r#"{"data":null}"#,
            },
            ProjectionCase {
                name: "strict / null inner list",
                field: "strict",
                input: r#"[null]"#,
                expected: r#"{"data":null}"#,
            },
            ProjectionCase {
                name: "strict / null outer list",
                field: "strict",
                input: r#"null"#,
                expected: r#"{"data":null}"#,
            },
            ProjectionCase {
                name: "strict / late null item",
                field: "strict",
                input: r#"[[{"__typename":"Foo","name":"a","nick":"b"},null]]"#,
                expected: r#"{"data":null}"#,
            },
            ProjectionCase {
                name: "strict / late null inner list",
                field: "strict",
                input: r#"[[{"__typename":"Foo","name":"a","nick":"b"}],null]"#,
                expected: r#"{"data":null}"#,
            },
            ProjectionCase {
                name: "nullable / valid",
                field: "nullable",
                input: r#"[[{"__typename":"Foo","name":"a","nick":"b"}]]"#,
                expected: r#"{"data":{"nullable":[[{"name":"a","nick":"b"}]]}}"#,
            },
            ProjectionCase {
                name: "nullable / nullable leaf",
                field: "nullable",
                input: r#"[[{"__typename":"Foo","name":"a","nick":null}]]"#,
                expected: r#"{"data":{"nullable":[[{"name":"a","nick":null}]]}}"#,
            },
            ProjectionCase {
                name: "nullable / non-null leaf",
                field: "nullable",
                input: r#"[[{"__typename":"Foo","name":null,"nick":"b"}]]"#,
                expected: r#"{"data":{"nullable":[[null]]}}"#,
            },
            ProjectionCase {
                name: "nullable / null object",
                field: "nullable",
                input: r#"[[null]]"#,
                expected: r#"{"data":{"nullable":[[null]]}}"#,
            },
            ProjectionCase {
                name: "nullable / null inner list",
                field: "nullable",
                input: r#"[[{"__typename":"Foo","name":"a","nick":"b"}],null]"#,
                expected: r#"{"data":{"nullable":[[{"name":"a","nick":"b"}],null]}}"#,
            },
            ProjectionCase {
                name: "nullable / null outer list",
                field: "nullable",
                input: r#"null"#,
                expected: r#"{"data":{"nullable":null}}"#,
            },
            ProjectionCase {
                name: "boundary / non-null leaf",
                field: "boundary",
                input: r#"[[{"__typename":"Foo","name":null,"nick":"b"}]]"#,
                expected: r#"{"data":{"boundary":null}}"#,
            },
            ProjectionCase {
                name: "boundary / null inner list",
                field: "boundary",
                input: r#"[null]"#,
                expected: r#"{"data":{"boundary":null}}"#,
            },
            ProjectionCase {
                name: "mixed / valid",
                field: "mixed",
                input: r#"[{"__typename":"Foo","name":"a","nick":"b"}]"#,
                expected: r#"{"data":{"mixed":[{"name":"a","nick":"b"}]}}"#,
            },
            ProjectionCase {
                name: "mixed / nullable leaf",
                field: "mixed",
                input: r#"[{"__typename":"Foo","name":"a","nick":null}]"#,
                expected: r#"{"data":{"mixed":[{"name":"a","nick":null}]}}"#,
            },
            ProjectionCase {
                name: "mixed / non-null leaf",
                field: "mixed",
                input: r#"[{"__typename":"Foo","name":null,"nick":"b"}]"#,
                expected: r#"{"data":null}"#,
            },
            ProjectionCase {
                name: "mixed / null object",
                field: "mixed",
                input: r#"[null]"#,
                expected: r#"{"data":null}"#,
            },
            ProjectionCase {
                name: "mixed / null outer list",
                field: "mixed",
                input: r#"null"#,
                expected: r#"{"data":null}"#,
            },
            ProjectionCase {
                name: "mixed / late null item",
                field: "mixed",
                input: r#"[{"__typename":"Foo","name":"a","nick":"b"},null]"#,
                expected: r#"{"data":null}"#,
            },
        ];
        for (index, case) in cases.iter().enumerate() {
            let data = format!(
                r#"{{"__typename":"Query","{}":{}}}"#,
                case.field, case.input
            );
            let actual = project(
                &supergraph_state,
                &schema_metadata,
                &format!(r#"query {{ {} {{ name nick }} }}"#, case.field),
                &data,
                None,
            );
            assert_eq!(actual, case.expected, "case {index}: {}", case.name);
        }
    }

    /// A requested non-null field that is **absent** from the subgraph data
    /// (not just explicit `null`) must still bubble, via the missing-field
    /// branch in `project_object_fields`. This is the shape a partially
    /// materialized entity (e.g. failed entity resolution, cf. #1110) can
    /// expose to the projector.
    #[test]
    fn missing_non_null_field_bubbles() {
        let supergraph = parse_schema(
            r#"
            type Query { strictUser: User!, nullableUser: User }
            type User { id: ID!, email: String! }
            "#,
        );
        let consumer_schema = ConsumerSchema::new_from_supergraph(&supergraph);
        let schema_metadata = consumer_schema.schema_metadata();
        let supergraph_state = SupergraphState::new(&supergraph);

        let cases = [
            ProjectionCase {
                name: "strict parent / missing email",
                field: "strictUser",
                input: r#"{"__typename":"User","id":"1"}"#,
                expected: r#"{"data":null}"#,
            },
            ProjectionCase {
                name: "nullable parent / missing email",
                field: "nullableUser",
                input: r#"{"__typename":"User","id":"1"}"#,
                expected: r#"{"data":{"nullableUser":null}}"#,
            },
            ProjectionCase {
                name: "fully materialized",
                field: "strictUser",
                input: r#"{"__typename":"User","id":"1","email":"a@x.test"}"#,
                expected: r#"{"data":{"strictUser":{"id":"1","email":"a@x.test"}}}"#,
            },
        ];
        for (index, case) in cases.iter().enumerate() {
            let data = format!(
                r#"{{"__typename":"Query","{}":{}}}"#,
                case.field, case.input
            );
            let actual = project(
                &supergraph_state,
                &schema_metadata,
                &format!(r#"query {{ {} {{ id email }} }}"#, case.field),
                &data,
                None,
            );
            assert_eq!(actual, case.expected, "case {index}: {}", case.name);
        }
    }

    /// Operations that normalize/filter to zero root fields must project to
    /// `{"data":{}}` instead of failing. Covers both the statically dropped
    /// case (`roots.len == 0` in the new `ProjectionPlan`) and the runtime
    /// `@skip` case where the plan is non-empty but every field is skipped.
    #[test]
    fn empty_projection_returns_empty_data_object() {
        let supergraph = parse_schema(
            r#"
            type Query { foo: String }
            "#,
        );
        let consumer_schema = ConsumerSchema::new_from_supergraph(&supergraph);
        let schema_metadata = consumer_schema.schema_metadata();
        let supergraph_state = SupergraphState::new(&supergraph);

        // Statically skipped: normalization drops the only field, so the plan
        // itself is empty.
        let operation = parse_operation(r#"query { foo @skip(if: true) }"#);
        let normalized = normalize_operation(&supergraph_state, &operation, None).unwrap();
        let (_, plan) =
            ProjectionPlan::from_operation(normalized.executable_operation(), &schema_metadata);
        assert!(
            plan.is_empty(),
            "statically skipped operation should produce an empty plan"
        );
        let static_output = project(
            &supergraph_state,
            &schema_metadata,
            r#"query { foo @skip(if: true) }"#,
            r#"{"__typename":"Query"}"#,
            None,
        );

        // Variable-skipped: the plan keeps the field behind a `Skip`
        // condition, and projection with `skip = true` must still yield `{}`.
        let operation =
            parse_operation(r#"query Example($skip: Boolean!) { foo @skip(if: $skip) }"#);
        let normalized = normalize_operation(&supergraph_state, &operation, None).unwrap();
        let (_, plan) =
            ProjectionPlan::from_operation(normalized.executable_operation(), &schema_metadata);
        assert!(
            !plan.is_empty(),
            "variable-skipped operation keeps its plan until execution"
        );
        let mut variables = HashMap::new();
        variables.insert("skip".to_string(), sonic_rs::Value::from(true));
        let variable_output = project(
            &supergraph_state,
            &schema_metadata,
            r#"query Example($skip: Boolean!) { foo @skip(if: $skip) }"#,
            r#"{"__typename":"Query","foo":"x"}"#,
            Some(variables),
        );

        for (index, (name, actual)) in [
            ("static plan", static_output),
            ("variable plan", variable_output),
        ]
        .into_iter()
        .enumerate()
        {
            assert_eq!(actual, r#"{"data":{}}"#, "case {index}: {name}");
        }
    }

    /// Nulling one concrete variant behind a shared response key must not
    /// leak into the sibling variant. Exercises `ProjectionPlan::rewrite`
    /// (authorization/operation-filter path) together with abstract-type
    /// specialization: only the `User`-guarded `secret` becomes null.
    #[test]
    fn authorization_rewrite_nulls_single_abstract_variant() {
        use crate::executor::operation_filter::PathSegment;
        use crate::pipeline::trie::Trie;
        use std::sync::Arc;

        let supergraph = parse_schema(
            r#"
            interface Node { id: ID! }
            type User implements Node { id: ID!, secret: String }
            type Admin implements Node { id: ID!, secret: String }
            type Query { node: Node }
            "#,
        );
        let consumer_schema = ConsumerSchema::new_from_supergraph(&supergraph);
        let schema_metadata = consumer_schema.schema_metadata();
        let supergraph_state = SupergraphState::new(&supergraph);
        let operation = parse_operation(
            r#"
            query {
              node {
                id
                ... on User { secret }
                ... on Admin { secret }
              }
            }
            "#,
        );
        let normalized = normalize_operation(&supergraph_state, &operation, None).unwrap();
        let (root_type_name, plan) =
            ProjectionPlan::from_operation(normalized.executable_operation(), &schema_metadata);
        let plan = Arc::new(plan);

        let project_variant = |plan: &ProjectionPlan, typename: &str| -> String {
            let data_json = sonic_rs::json!({"__typename": "Query", "node": {"__typename": typename, "id": "1", "secret": "shh"}});
            let data = Value::from(data_json.as_ref());
            project_plan(&data, root_type_name, plan, None, &schema_metadata)
        };
        let before_user = project_variant(&plan, "User");
        let before_admin = project_variant(&plan, "Admin");

        // Deny only `User.secret`: path is node -> User fragment -> secret.
        let trie = Trie::from_paths(&[vec![
            PathSegment::Field("node"),
            PathSegment::Fragment("User"),
            PathSegment::Field("secret"),
        ]]);
        let rewritten = plan.rewrite(&trie);

        let after_user = project_variant(&rewritten, "User");
        let after_admin = project_variant(&rewritten, "Admin");

        for (index, (name, actual, expected)) in [
            (
                "before User",
                &before_user,
                r#"{"data":{"node":{"id":"1","secret":"shh"}}}"#,
            ),
            (
                "before Admin",
                &before_admin,
                r#"{"data":{"node":{"id":"1","secret":"shh"}}}"#,
            ),
            (
                "after User",
                &after_user,
                r#"{"data":{"node":{"id":"1","secret":null}}}"#,
            ),
            (
                "after Admin",
                &after_admin,
                r#"{"data":{"node":{"id":"1","secret":"shh"}}}"#,
            ),
        ]
        .into_iter()
        .enumerate()
        {
            assert_eq!(actual, expected, "case {index}: {name}");
        }
    }
}
