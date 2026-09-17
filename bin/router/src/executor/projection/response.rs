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
    use crate::executor::introspection::schema::SchemaWithMetadata;
    use crate::query_planner::ast::normalization::normalize_operation;
    use crate::query_planner::consumer_schema::ConsumerSchema;
    use crate::query_planner::utils::parsing::{parse_operation, parse_schema};
    use crate::query_planner::{planner::Planner, state::supergraph_state::SupergraphState};

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
    ///
    /// Covers `[[Foo!]!]!`, `[[Foo]]`, `[[Foo!]!]` and `[Foo!]!`
    /// with `null` introduced at every level:
    /// - nullable leaf field
    /// - non-null leaf field
    /// - null object
    /// - null inner list
    /// - null outer list
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

        // Shape bytes are one flag byte per level, outside in:
        // LIST = 1, NON_NULL = 2.
        let assert_projection = |field: &str,
                                 expected_shape: &[u8],
                                 data_value: &str,
                                 expected: &str| {
            let operation = parse_operation(&format!(r#"query {{ {field} {{ name nick }} }}"#));
            let normalized = normalize_operation(&supergraph_state, &operation, None).unwrap();
            let (root_type_name, plan) =
                ProjectionPlan::from_operation(normalized.executable_operation(), &schema_metadata);
            let root_fields = plan.root_fields();
            assert_eq!(root_fields.len(), 1, "one root field for {field}");
            assert_eq!(
                plan.shape(root_fields[0].nullability()),
                expected_shape,
                "nullability shape for {field}"
            );

            let data_str = format!(r#"{{"__typename":"Query","{field}":{data_value}}}"#);
            let data_json: sonic_rs::Value = sonic_rs::from_str(&data_str).unwrap();
            let data = Value::from(data_json.as_ref());
            let output = project_by_operation(
                &data,
                vec![],
                &Default::default(),
                root_type_name,
                &plan,
                &None,
                1024,
                &schema_metadata,
            )
            .unwrap();
            assert_eq!(
                String::from_utf8(output).unwrap(),
                expected,
                "field {field} with data {data_value}"
            );
        };

        // `[[Foo!]!]!`: every level is non-null, so any `null` below the field
        // collapses all the way to `data`.
        let strict_shape: &[u8] = &[3, 3, 2];
        assert_projection(
            "strict",
            strict_shape,
            r#"[[{"__typename":"Foo","name":"a","nick":"b"}]]"#,
            r#"{"data":{"strict":[[{"name":"a","nick":"b"}]]}}"#,
        );
        // Nullable leaf stays put.
        assert_projection(
            "strict",
            strict_shape,
            r#"[[{"__typename":"Foo","name":"a","nick":null}]]"#,
            r#"{"data":{"strict":[[{"name":"a","nick":null}]]}}"#,
        );
        // Non-null leaf bubbles through both lists and the field.
        assert_projection(
            "strict",
            strict_shape,
            r#"[[{"__typename":"Foo","name":null,"nick":"b"}]]"#,
            r#"{"data":null}"#,
        );
        // Null object (`Foo!`) bubbles.
        assert_projection("strict", strict_shape, r#"[[null]]"#, r#"{"data":null}"#);
        // Null inner list (`[Foo!]!`) bubbles.
        assert_projection("strict", strict_shape, r#"[null]"#, r#"{"data":null}"#);
        // Null outer list bubbles through the non-null field.
        assert_projection("strict", strict_shape, r#"null"#, r#"{"data":null}"#);
        // A late bad item discards earlier good items at the inner level.
        assert_projection(
            "strict",
            strict_shape,
            r#"[[{"__typename":"Foo","name":"a","nick":"b"},null]]"#,
            r#"{"data":null}"#,
        );
        // A late bad inner discards earlier good inners at the outer level.
        assert_projection(
            "strict",
            strict_shape,
            r#"[[{"__typename":"Foo","name":"a","nick":"b"}],null]"#,
            r#"{"data":null}"#,
        );

        // `[[Foo]]`: every level is nullable, so `null` stays where it appears.
        let nullable_shape: &[u8] = &[1, 1, 0];
        assert_projection(
            "nullable",
            nullable_shape,
            r#"[[{"__typename":"Foo","name":"a","nick":"b"}]]"#,
            r#"{"data":{"nullable":[[{"name":"a","nick":"b"}]]}}"#,
        );
        assert_projection(
            "nullable",
            nullable_shape,
            r#"[[{"__typename":"Foo","name":"a","nick":null}]]"#,
            r#"{"data":{"nullable":[[{"name":"a","nick":null}]]}}"#,
        );
        // Non-null leaf still nulls its own (nullable) object, but no further.
        assert_projection(
            "nullable",
            nullable_shape,
            r#"[[{"__typename":"Foo","name":null,"nick":"b"}]]"#,
            r#"{"data":{"nullable":[[null]]}}"#,
        );
        assert_projection(
            "nullable",
            nullable_shape,
            r#"[[null]]"#,
            r#"{"data":{"nullable":[[null]]}}"#,
        );
        // Null inner list stays next to its good sibling.
        assert_projection(
            "nullable",
            nullable_shape,
            r#"[[{"__typename":"Foo","name":"a","nick":"b"}],null]"#,
            r#"{"data":{"nullable":[[{"name":"a","nick":"b"}],null]}}"#,
        );
        assert_projection(
            "nullable",
            nullable_shape,
            r#"null"#,
            r#"{"data":{"nullable":null}}"#,
        );

        // `[[Foo!]!]`: bubbling crosses the non-null inner list and outer
        // items, then stops at the nullable outer list / field.
        let boundary_shape: &[u8] = &[1, 3, 2];
        assert_projection(
            "boundary",
            boundary_shape,
            r#"[[{"__typename":"Foo","name":null,"nick":"b"}]]"#,
            r#"{"data":{"boundary":null}}"#,
        );
        // The inner list is a non-null item of the outer list.
        assert_projection(
            "boundary",
            boundary_shape,
            r#"[null]"#,
            r#"{"data":{"boundary":null}}"#,
        );

        // `[Foo!]!`: outer list and items are non-null, so item `null`s bubble
        // to `data` while nullable leaves stay put.
        let mixed_shape: &[u8] = &[3, 2];
        assert_projection(
            "mixed",
            mixed_shape,
            r#"[{"__typename":"Foo","name":"a","nick":"b"}]"#,
            r#"{"data":{"mixed":[{"name":"a","nick":"b"}]}}"#,
        );
        assert_projection(
            "mixed",
            mixed_shape,
            r#"[{"__typename":"Foo","name":"a","nick":null}]"#,
            r#"{"data":{"mixed":[{"name":"a","nick":null}]}}"#,
        );
        assert_projection(
            "mixed",
            mixed_shape,
            r#"[{"__typename":"Foo","name":null,"nick":"b"}]"#,
            r#"{"data":null}"#,
        );
        assert_projection("mixed", mixed_shape, r#"[null]"#, r#"{"data":null}"#);
        assert_projection("mixed", mixed_shape, r#"null"#, r#"{"data":null}"#);
        assert_projection(
            "mixed",
            mixed_shape,
            r#"[{"__typename":"Foo","name":"a","nick":"b"},null]"#,
            r#"{"data":null}"#,
        );
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

        let project = |field: &str, data_value: &str| -> String {
            let operation = parse_operation(&format!(r#"query {{ {field} {{ id email }} }}"#));
            let normalized = normalize_operation(&supergraph_state, &operation, None).unwrap();
            let (root_type_name, plan) =
                ProjectionPlan::from_operation(normalized.executable_operation(), &schema_metadata);
            let data_str = format!(r#"{{"__typename":"Query","{field}":{data_value}}}"#);
            let data_json: sonic_rs::Value = sonic_rs::from_str(&data_str).unwrap();
            let data = Value::from(data_json.as_ref());
            let output = project_by_operation(
                &data,
                vec![],
                &Default::default(),
                root_type_name,
                &plan,
                &None,
                256,
                &schema_metadata,
            )
            .unwrap();
            String::from_utf8(output).unwrap()
        };

        // `email` absent under a non-null chain collapses all the way to `data`.
        assert_eq!(
            project("strictUser", r#"{"__typename":"User","id":"1"}"#,),
            r#"{"data":null}"#,
        );
        // Same absence under a nullable parent stops at that parent.
        assert_eq!(
            project("nullableUser", r#"{"__typename":"User","id":"1"}"#,),
            r#"{"data":{"nullableUser":null}}"#,
        );
        // Sanity: a fully materialized object projects normally.
        assert_eq!(
            project(
                "strictUser",
                r#"{"__typename":"User","id":"1","email":"a@x.test"}"#,
            ),
            r#"{"data":{"strictUser":{"id":"1","email":"a@x.test"}}}"#,
        );
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
        let (root_type_name, plan) =
            ProjectionPlan::from_operation(normalized.executable_operation(), &schema_metadata);
        assert!(
            plan.is_empty(),
            "statically skipped operation should produce an empty plan"
        );
        let data_json = sonic_rs::json!({"__typename": "Query"});
        let data = Value::from(data_json.as_ref());
        let output = project_by_operation(
            &data,
            vec![],
            &Default::default(),
            root_type_name,
            &plan,
            &None,
            64,
            &schema_metadata,
        )
        .unwrap();
        assert_eq!(String::from_utf8(output).unwrap(), r#"{"data":{}}"#);

        // Variable-skipped: the plan keeps the field behind a `Skip`
        // condition, and projection with `skip = true` must still yield `{}`.
        let operation =
            parse_operation(r#"query Example($skip: Boolean!) { foo @skip(if: $skip) }"#);
        let normalized = normalize_operation(&supergraph_state, &operation, None).unwrap();
        let (root_type_name, plan) =
            ProjectionPlan::from_operation(normalized.executable_operation(), &schema_metadata);
        assert!(
            !plan.is_empty(),
            "variable-skipped operation keeps its plan until execution"
        );
        let mut variables = HashMap::new();
        variables.insert("skip".to_string(), sonic_rs::Value::from(true));
        let data_json = sonic_rs::json!({"__typename": "Query", "foo": "x"});
        let data = Value::from(data_json.as_ref());
        let output = project_by_operation(
            &data,
            vec![],
            &Default::default(),
            root_type_name,
            &plan,
            &Some(variables),
            64,
            &schema_metadata,
        )
        .unwrap();
        assert_eq!(String::from_utf8(output).unwrap(), r#"{"data":{}}"#);
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

        // Sanity: both variants project before the rewrite.
        let project = |plan: &ProjectionPlan, typename: &str| -> String {
            let data_json = sonic_rs::json!({"__typename": "Query", "node": {"__typename": typename, "id": "1", "secret": "shh"}});
            let data = Value::from(data_json.as_ref());
            let output = project_by_operation(
                &data,
                vec![],
                &Default::default(),
                root_type_name,
                plan,
                &None,
                256,
                &schema_metadata,
            )
            .unwrap();
            String::from_utf8(output).unwrap()
        };
        assert_eq!(
            project(&plan, "User"),
            r#"{"data":{"node":{"id":"1","secret":"shh"}}}"#
        );
        assert_eq!(
            project(&plan, "Admin"),
            r#"{"data":{"node":{"id":"1","secret":"shh"}}}"#
        );

        // Deny only `User.secret`: path is node -> User fragment -> secret.
        let trie = Trie::from_paths(&[vec![
            PathSegment::Field("node"),
            PathSegment::Fragment("User"),
            PathSegment::Field("secret"),
        ]]);
        let rewritten = plan.rewrite(&trie);

        assert_eq!(
            project(&rewritten, "User"),
            r#"{"data":{"node":{"id":"1","secret":null}}}"#,
            "nulled User variant must render secret as null"
        );
        assert_eq!(
            project(&rewritten, "Admin"),
            r#"{"data":{"node":{"id":"1","secret":"shh"}}}"#,
            "sibling Admin variant must be unaffected"
        );
    }
}
