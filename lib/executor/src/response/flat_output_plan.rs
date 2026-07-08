use std::collections::HashMap;
use std::sync::Arc;

use bytes::BufMut;
use sonic_rs::JsonValueTrait;

use crate::introspection::schema::{FieldNullability, PossibleTypes};
use crate::json_writer::{write_and_escape_string, write_f64, write_i64, write_u64};
use crate::projection::plan::{
    FieldProjectionCondition, FieldProjectionPlan, ProjectionValueSource, TypeCondition,
};
use crate::response::flat_store::{
    FlatResponseStore, FlatValue, FlatValueId, ResponseKeyId, ResponseKeys,
};
use crate::utils::consts::CLOSE_BRACE as CLOSE_BRACE_;
use crate::utils::consts::CLOSE_BRACKET as CLOSE_BRACKET_;
use crate::utils::consts::OPEN_BRACE as OPEN_BRACE_;
use crate::utils::consts::OPEN_BRACKET as OPEN_BRACKET_;
use crate::utils::consts::*;

pub type VariablesMap = HashMap<String, sonic_rs::Value>;

/// Compiled output plan that describes the final response layout.
#[derive(Debug, Clone)]
pub struct FlatOutputPlan {
    pub keys: Arc<ResponseKeys>,
    pub root_fields: Vec<FlatOutputField>,
}

/// One output field in the final response layout.
#[derive(Debug, Clone)]
pub struct FlatOutputField {
    pub response_key_id: ResponseKeyId,
    pub source_key: Box<str>,
    pub nullability: FieldNullability,
    pub kind: FlatOutputFieldKind,
    pub condition: Option<FlatCondition>,
    pub is_typename: bool,
}

#[derive(Debug, Clone)]
pub enum FlatOutputFieldKind {
    Scalar,
    CustomScalar,
    Object { children: Vec<FlatOutputField> },
    List { item: Option<Box<FlatOutputField>> },
}

#[derive(Debug, Clone)]
pub enum FlatCondition {
    IncludeIfVar(String),
    SkipIfVar(String),
    ParentTypeCondition(TypeCondition),
    FieldTypeCondition(TypeCondition),
    EnumValuesCondition(Vec<String>),
    And(Box<FlatCondition>, Box<FlatCondition>),
    Or(Box<FlatCondition>, Box<FlatCondition>),
}

enum FlatConditionError {
    Skip,
    InvalidParentType,
    InvalidFieldType,
    InvalidEnumValue,
}

// ---------------------------------------------------------------------------
// Compiler
// ---------------------------------------------------------------------------

impl FlatOutputPlan {
    pub fn compile(projection_plan: &[FieldProjectionPlan]) -> Self {
        let mut keys = ResponseKeys::default();
        let root_fields = projection_plan
            .iter()
            .map(|plan| compile_output_field(plan, &mut keys))
            .collect();
        FlatOutputPlan {
            keys: Arc::new(keys),
            root_fields,
        }
    }
}

fn compile_output_field(plan: &FieldProjectionPlan, keys: &mut ResponseKeys) -> FlatOutputField {
    let response_key_id = keys.intern(&plan.response_key);
    let source_key: Box<str> = plan.field_name.clone().into_boxed_str();
    let condition = compile_optional_condition(&plan.conditions, plan.parent_type_guard.as_ref());

    let kind = match &plan.nullability {
        FieldNullability::List {
            item: item_nullability,
            ..
        } => {
            let item_field =
                compile_item_field(item_nullability, &plan.value, plan.is_typename, keys);
            FlatOutputFieldKind::List {
                item: Some(Box::new(item_field)),
            }
        }
        FieldNullability::Leaf { .. } => compile_leaf_kind(&plan.value, plan.is_typename, keys),
    };

    FlatOutputField {
        response_key_id,
        source_key,
        nullability: plan.nullability.clone(),
        kind,
        condition,
        is_typename: plan.is_typename,
    }
}

fn compile_leaf_kind(
    value: &ProjectionValueSource,
    is_typename: bool,
    keys: &mut ResponseKeys,
) -> FlatOutputFieldKind {
    match value {
        ProjectionValueSource::Null => FlatOutputFieldKind::Scalar,
        ProjectionValueSource::ResponseData { selections: None } => {
            let _ = (is_typename, keys);
            FlatOutputFieldKind::Scalar
        }
        ProjectionValueSource::ResponseData {
            selections: Some(children),
        } => {
            let compiled = children
                .iter()
                .map(|child| compile_output_field(child, keys))
                .collect();
            FlatOutputFieldKind::Object { children: compiled }
        }
    }
}

fn compile_item_field(
    nullability: &FieldNullability,
    value: &ProjectionValueSource,
    is_typename: bool,
    keys: &mut ResponseKeys,
) -> FlatOutputField {
    let dummy_key_id = keys.intern("");
    let kind = match nullability {
        FieldNullability::List { item: inner, .. } => {
            let inner_field = compile_item_field(inner, value, is_typename, keys);
            FlatOutputFieldKind::List {
                item: Some(Box::new(inner_field)),
            }
        }
        FieldNullability::Leaf { .. } => compile_leaf_kind(value, is_typename, keys),
    };

    FlatOutputField {
        response_key_id: dummy_key_id,
        source_key: Box::from(""),
        nullability: nullability.clone(),
        kind,
        condition: None,
        is_typename,
    }
}

fn compile_optional_condition(
    conditions: &Option<FieldProjectionCondition>,
    parent_type_guard: Option<&TypeCondition>,
) -> Option<FlatCondition> {
    match (conditions.as_ref(), parent_type_guard) {
        (Some(cond), Some(guard)) => Some(FlatCondition::And(
            Box::new(FlatCondition::ParentTypeCondition(guard.clone())),
            Box::new(compile_condition(cond)),
        )),
        (Some(cond), None) => Some(compile_condition(cond)),
        (None, Some(guard)) => Some(FlatCondition::ParentTypeCondition(guard.clone())),
        (None, None) => None,
    }
}

fn compile_condition(cond: &FieldProjectionCondition) -> FlatCondition {
    match cond {
        FieldProjectionCondition::IncludeIfVariable(v) => FlatCondition::IncludeIfVar(v.clone()),
        FieldProjectionCondition::SkipIfVariable(v) => FlatCondition::SkipIfVar(v.clone()),
        FieldProjectionCondition::ParentTypeCondition(tc) => {
            FlatCondition::ParentTypeCondition(tc.clone())
        }
        FieldProjectionCondition::FieldTypeCondition(tc) => {
            FlatCondition::FieldTypeCondition(tc.clone())
        }
        FieldProjectionCondition::EnumValuesCondition(ev) => {
            FlatCondition::EnumValuesCondition(ev.iter().cloned().collect())
        }
        FieldProjectionCondition::Or(a, b) => FlatCondition::Or(
            Box::new(compile_condition(a)),
            Box::new(compile_condition(b)),
        ),
        FieldProjectionCondition::And(a, b) => FlatCondition::And(
            Box::new(compile_condition(a)),
            Box::new(compile_condition(b)),
        ),
    }
}

// ---------------------------------------------------------------------------
// Condition evaluator
// ---------------------------------------------------------------------------

fn check_flat_condition_with_value(
    cond: &FlatCondition,
    variable_values: &Option<VariablesMap>,
    parent_type_name: Option<&str>,
    field_value: Option<&FlatValue>,
    possible_types: &PossibleTypes,
) -> Result<(), FlatConditionError> {
    match cond {
        FlatCondition::And(a, b) => check_flat_condition_with_value(
            a,
            variable_values,
            parent_type_name,
            field_value,
            possible_types,
        )
        .and_then(|_| {
            check_flat_condition_with_value(
                b,
                variable_values,
                parent_type_name,
                field_value,
                possible_types,
            )
        }),
        FlatCondition::Or(a, b) => check_flat_condition_with_value(
            a,
            variable_values,
            parent_type_name,
            field_value,
            possible_types,
        )
        .or_else(|_| {
            check_flat_condition_with_value(
                b,
                variable_values,
                parent_type_name,
                field_value,
                possible_types,
            )
        }),
        FlatCondition::IncludeIfVar(var) => {
            let is_truthy = variable_values
                .as_ref()
                .and_then(|vv| vv.get(var))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if is_truthy {
                Ok(())
            } else {
                Err(FlatConditionError::Skip)
            }
        }
        FlatCondition::SkipIfVar(var) => {
            let is_truthy = variable_values
                .as_ref()
                .and_then(|vv| vv.get(var))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if is_truthy {
                Err(FlatConditionError::Skip)
            } else {
                Ok(())
            }
        }
        FlatCondition::ParentTypeCondition(type_condition) => {
            if let Some(parent_type) = parent_type_name {
                if type_condition.matches(parent_type) {
                    Ok(())
                } else {
                    Err(FlatConditionError::InvalidParentType)
                }
            } else {
                Ok(())
            }
        }
        FlatCondition::FieldTypeCondition(type_condition) => {
            let field_type = resolve_flat_value_type_name(field_value);
            if let Some(ft) = field_type {
                if type_condition.matches(&ft) {
                    Ok(())
                } else {
                    Err(FlatConditionError::InvalidFieldType)
                }
            } else {
                Ok(())
            }
        }
        FlatCondition::EnumValuesCondition(allowed_values) => {
            let is_allowed = field_value.map_or(true, |fv| match fv {
                FlatValue::String(s) => allowed_values.iter().any(|av| av.as_str() == s.as_ref()),
                _ => true,
            });
            if is_allowed {
                Ok(())
            } else {
                Err(FlatConditionError::InvalidEnumValue)
            }
        }
    }
}

fn resolve_flat_value_type_name(value: Option<&FlatValue>) -> Option<String> {
    value.and_then(|fv| match fv {
        FlatValue::String(s) => Some(s.as_ref().to_string()),
        _ => None,
    })
}

// ---------------------------------------------------------------------------
// Serializer
// ---------------------------------------------------------------------------

/// Serialize the flat response using the compiled output plan.
///
/// This replaces raw flat serialization with a projection-aware serializer
/// that observes field order, nullability, conditions, and type guards.
pub fn serialize_with_output_plan(
    store: &FlatResponseStore,
    plan: &FlatOutputPlan,
    flat_keys: &ResponseKeys,
    root: FlatValueId,
    variable_values: &Option<VariablesMap>,
    possible_types: &PossibleTypes,
    root_type_name: &str,
    buffer: &mut Vec<u8>,
) {
    let _ = serialize_output_object(
        store,
        &plan.keys,
        flat_keys,
        &plan.root_fields,
        root,
        variable_values,
        possible_types,
        Some(root_type_name),
        buffer,
    );
}

/// Returns `true` if the serialization succeeded without non-null violation.
/// Returns `false` if a non-null field received null; the caller must truncate
/// to `checkpoint` and write `null`.
fn serialize_output_object(
    store: &FlatResponseStore,
    output_keys: &ResponseKeys,
    flat_keys: &ResponseKeys,
    children: &[FlatOutputField],
    object_id: FlatValueId,
    variable_values: &Option<VariablesMap>,
    possible_types: &PossibleTypes,
    parent_type_name: Option<&str>,
    buffer: &mut Vec<u8>,
) -> bool {
    let obj_range = match store.value(object_id) {
        FlatValue::Object { fields } => fields.clone(),
        _ => {
            // Not an object – serialize as null
            buffer.put(NULL);
            return false;
        }
    };

    // Resolve the actual parent type name from __typename. For entity objects,
    // __typename is always present. Fall back to the caller-provided hint then "Query".
    let resolved_parent = flat_get_typename(store, flat_keys, &obj_range)
        .or_else(|| parent_type_name.map(|s| s.to_string()))
        .unwrap_or_else(|| "Query".to_string());

    let checkpoint = buffer.len();
    buffer.put(OPEN_BRACE_);
    let mut first = true;

    for child in children {
        let condition_result = child.condition.as_ref().map(|cond| {
            check_flat_condition_with_value(
                cond,
                variable_values,
                Some(&resolved_parent),
                None,
                possible_types,
            )
        });

        match condition_result {
            Some(Err(FlatConditionError::Skip | FlatConditionError::InvalidParentType)) => {
                continue;
            }
            Some(Err(
                FlatConditionError::InvalidFieldType | FlatConditionError::InvalidEnumValue,
            )) => {
                // Write null for this field
                if !first {
                    buffer.put(COMMA);
                }
                first = false;
                buffer.put_slice(output_keys.serialized_json_key(child.response_key_id));
                buffer.put(NULL);
                if child.nullability.is_non_null() {
                    buffer.truncate(checkpoint);
                    buffer.put(NULL);
                    return false;
                }
                continue;
            }
            _ => { /* Ok or None */ }
        }

        // For __typename fields, serialize the resolved parent type name
        if child.is_typename {
            if !first {
                buffer.put(COMMA);
            }
            first = false;
            buffer.put_slice(output_keys.serialized_json_key(child.response_key_id));
            write_and_escape_string(buffer, &resolved_parent);
            continue;
        }

        // Final projection reads merged response objects by response key, matching
        // the Value-based projection path.
        let response_key = output_keys.key(child.response_key_id);
        let field_value_id = flat_keys
            .get_key_id(response_key)
            .and_then(|kid| find_field_in_flat_object(store, &obj_range, kid));

        // Now we need to evaluate conditions that depend on the field value
        // (EnumValuesCondition) - re-evaluate if not already done
        let condition_result_with_value = child.condition.as_ref().map(|cond| {
            let field_val = field_value_id.map(|id| store.value(id));
            check_flat_condition_with_value(
                cond,
                variable_values,
                Some(&resolved_parent),
                field_val,
                possible_types,
            )
        });

        match condition_result_with_value {
            Some(Err(FlatConditionError::Skip | FlatConditionError::InvalidParentType)) => {
                continue;
            }
            Some(Err(
                FlatConditionError::InvalidFieldType | FlatConditionError::InvalidEnumValue,
            )) => {
                if !first {
                    buffer.put(COMMA);
                }
                first = false;
                buffer.put_slice(output_keys.serialized_json_key(child.response_key_id));
                buffer.put(NULL);
                if child.nullability.is_non_null() {
                    buffer.truncate(checkpoint);
                    buffer.put(NULL);
                    return false;
                }
                continue;
            }
            _ => { /* Ok */ }
        }

        if !first {
            buffer.put(COMMA);
        }
        first = false;

        buffer.put_slice(output_keys.serialized_json_key(child.response_key_id));

        let ok = match field_value_id {
            Some(value_id) => serialize_output_value(
                store,
                output_keys,
                flat_keys,
                child,
                value_id,
                variable_values,
                possible_types,
                Some(&resolved_parent),
                buffer,
            ),
            None => {
                buffer.put(NULL);
                false // field is missing = null → propagate
            }
        };

        if !ok && child.nullability.is_non_null() {
            buffer.truncate(checkpoint);
            buffer.put(NULL);
            return false;
        }
    }

    buffer.put(CLOSE_BRACE_);
    true
}

/// Returns `true` if the value was serialized without non-null violation.
fn serialize_output_value(
    store: &FlatResponseStore,
    output_keys: &ResponseKeys,
    flat_keys: &ResponseKeys,
    field: &FlatOutputField,
    value_id: FlatValueId,
    variable_values: &Option<VariablesMap>,
    possible_types: &PossibleTypes,
    field_parent_type: Option<&str>,
    buffer: &mut Vec<u8>,
) -> bool {
    match &field.kind {
        FlatOutputFieldKind::Scalar | FlatOutputFieldKind::CustomScalar => {
            serialize_flat_scalar(store, value_id, buffer);
            !is_propagating_null(store, value_id)
        }
        FlatOutputFieldKind::Object { children } => serialize_output_object(
            store,
            output_keys,
            flat_keys,
            children,
            value_id,
            variable_values,
            possible_types,
            field_parent_type,
            buffer,
        ),
        FlatOutputFieldKind::List {
            item: Some(item_field),
        } => serialize_output_list(
            store,
            output_keys,
            flat_keys,
            field,
            item_field,
            value_id,
            variable_values,
            possible_types,
            field_parent_type,
            buffer,
        ),
        FlatOutputFieldKind::List { item: None } => {
            // Empty list with no item type – serialize as empty array
            buffer.put(OPEN_BRACKET_);
            buffer.put(CLOSE_BRACKET_);
            true
        }
    }
}

/// Returns `true` if the list was serialized without non-null violation.
fn serialize_output_list(
    store: &FlatResponseStore,
    output_keys: &ResponseKeys,
    flat_keys: &ResponseKeys,
    field: &FlatOutputField,
    item_field: &FlatOutputField,
    list_value_id: FlatValueId,
    variable_values: &Option<VariablesMap>,
    possible_types: &PossibleTypes,
    _list_parent_type: Option<&str>,
    buffer: &mut Vec<u8>,
) -> bool {
    let item_range = match store.value(list_value_id) {
        FlatValue::RawJson(_) => {
            serialize_flat_scalar(store, list_value_id, buffer);
            return true;
        }
        FlatValue::List { items } => items.clone(),
        _ => {
            buffer.put(NULL);
            return false;
        }
    };

    let item_nullability = field.nullability.list_item();
    let item_non_null = item_nullability.is_some_and(FieldNullability::is_non_null);

    let checkpoint = buffer.len();
    buffer.put(OPEN_BRACKET_);

    let items: Vec<FlatValueId> = store.list_items(&item_range).to_vec();
    if items.is_empty() {
        buffer.put(CLOSE_BRACKET_);
        return true;
    }

    let mut first = true;
    for &item_id in &items {
        if !first {
            buffer.put(COMMA);
        }
        first = false;

        let ok = serialize_output_value(
            store,
            output_keys,
            flat_keys,
            item_field,
            item_id,
            variable_values,
            possible_types,
            None,
            buffer,
        );

        if !ok && item_non_null {
            buffer.truncate(checkpoint);
            buffer.put(NULL);
            return false;
        }
    }

    buffer.put(CLOSE_BRACKET_);
    true
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn find_field_in_flat_object(
    store: &FlatResponseStore,
    range: &std::ops::Range<u32>,
    key_id: ResponseKeyId,
) -> Option<FlatValueId> {
    store
        .object_fields(range)
        .iter()
        .find(|f| f.response_key == key_id)
        .map(|f| f.value)
}

fn flat_get_typename(
    store: &FlatResponseStore<'_>,
    flat_keys: &ResponseKeys,
    obj_range: &std::ops::Range<u32>,
) -> Option<String> {
    let typename_key_id = flat_keys.get_key_id("__typename")?;
    for sf in store.object_fields(obj_range) {
        if sf.response_key == typename_key_id {
            if let FlatValue::String(s) = store.value(sf.value) {
                return Some(s.to_string());
            }
        }
    }
    None
}

fn serialize_flat_scalar(store: &FlatResponseStore, id: FlatValueId, buffer: &mut Vec<u8>) {
    match store.value(id) {
        FlatValue::Null | FlatValue::Missing | FlatValue::Inaccessible => buffer.put(NULL),
        FlatValue::Bool(true) => buffer.put(TRUE),
        FlatValue::Bool(false) => buffer.put(FALSE),
        FlatValue::I64(n) => write_i64(buffer, *n),
        FlatValue::U64(n) => write_u64(buffer, *n),
        FlatValue::F64(n) => write_f64(buffer, *n),
        FlatValue::String(s) => write_and_escape_string(buffer, s),
        FlatValue::RawJson(raw) => buffer.put_slice(raw.as_bytes()),
        _ => buffer.put(NULL),
    }
}

fn is_propagating_null(store: &FlatResponseStore, id: FlatValueId) -> bool {
    matches!(
        store.value(id),
        FlatValue::Null | FlatValue::Missing | FlatValue::Inaccessible
    )
}
