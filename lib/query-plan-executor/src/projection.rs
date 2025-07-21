use std::collections::{HashMap, HashSet};
// use std::fmt::Write;
use std::io::Write;

use indexmap::IndexMap;
use query_planner::{
    ast::{
        operation::OperationDefinition, selection_item::SelectionItem, selection_set::SelectionSet,
    },
    state::supergraph_state::OperationKind,
};
use tracing::{instrument, warn};

use crate::json_writer::write_and_escape_string_writer;
use crate::responses::Value;
use crate::{
    json_writer::write_and_escape_string, schema_metadata::SchemaMetadata, GraphQLError,
    TYPENAME_FIELD,
};

#[derive(Debug)]
pub struct FieldProjectionPlan {
    field_name: String,
    field_type: String,
    response_key: String,
    conditions: FieldProjectionCondition,
    selections: Option<Vec<FieldProjectionPlan>>,
}

#[derive(Debug, Clone)]
pub enum FieldProjectionCondition {
    IncludeIfVariable(String),
    SkipIfVariable(String),
    ParentTypeCondition(HashSet<String>),
    FieldTypeCondition(HashSet<String>),
    EnumValuesCondition(HashSet<String>),
    Or(Box<FieldProjectionCondition>, Box<FieldProjectionCondition>),
    And(Box<FieldProjectionCondition>, Box<FieldProjectionCondition>),
}

pub enum FieldProjectionConditionError {
    InvalidParentType,
    InvalidFieldType,
    Skip,
    InvalidEnumValue,
}

impl FieldProjectionCondition {
    pub fn check(
        &self,
        parent_type_name: &str,
        field_type_name: &str,
        field_value: &Option<&Value>,
        variable_values: &Option<HashMap<String, serde_json::Value>>,
    ) -> Result<(), FieldProjectionConditionError> {
        match self {
            FieldProjectionCondition::And(condition_a, condition_b) => condition_a
                .check(
                    parent_type_name,
                    field_type_name,
                    field_value,
                    variable_values,
                )
                .and(condition_b.check(
                    parent_type_name,
                    field_type_name,
                    field_value,
                    variable_values,
                )),
            FieldProjectionCondition::Or(condition_a, condition_b) => condition_a
                .check(
                    parent_type_name,
                    field_type_name,
                    field_value,
                    variable_values,
                )
                .or(condition_b.check(
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
                    .and_then(|value| value.iter().find(|(k, _)| k == &TYPENAME_FIELD))
                    .and_then(|(_, v)| v.as_str())
                    .unwrap_or(field_type_name);
                if possible_types.contains(field_type_name) {
                    Ok(())
                } else {
                    Err(FieldProjectionConditionError::InvalidFieldType)
                }
            }
            FieldProjectionCondition::EnumValuesCondition(enum_values) => {
                if let Some(Value::String(string_value)) = field_value {
                    if enum_values.contains(*string_value) {
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
}

impl FieldProjectionPlan {
    pub fn from_selection_set(
        selection_set: &SelectionSet,
        schema_metadata: &SchemaMetadata,
        type_name: &str,
        condition: &FieldProjectionCondition,
    ) -> Option<Vec<FieldProjectionPlan>> {
        let mut field_selections: IndexMap<String, FieldProjectionPlan> = IndexMap::new();
        for selection_item in &selection_set.items {
            match selection_item {
                SelectionItem::Field(field) => {
                    let field_name = &field.name;
                    let response_key = field.alias.as_ref().unwrap_or(field_name);
                    let field_type = if field_name == TYPENAME_FIELD {
                        "String"
                    } else {
                        let field_map = match schema_metadata.type_fields.get(type_name) {
                            Some(fields) => fields,
                            None => {
                                warn!("No fields found for type {} in schema metadata.", type_name);
                                return None;
                            }
                        };
                        match field_map.get(field_name) {
                            Some(field_type) => field_type,
                            None => {
                                warn!(
                                    "Field {} not found in type {} in schema metadata.",
                                    field_name, type_name
                                );
                                continue;
                            }
                        }
                    };

                    let possible_types_for_field = schema_metadata
                        .possible_types
                        .get_possible_types(field_type);
                    let mut conditions_for_selections =
                        FieldProjectionCondition::ParentTypeCondition(
                            possible_types_for_field.clone(),
                        );
                    if let Some(include_if) = &field.include_if {
                        conditions_for_selections = FieldProjectionCondition::And(
                            Box::new(conditions_for_selections),
                            Box::new(FieldProjectionCondition::IncludeIfVariable(
                                include_if.clone(),
                            )),
                        );
                    }
                    if let Some(skip_if) = &field.skip_if {
                        conditions_for_selections = FieldProjectionCondition::And(
                            Box::new(conditions_for_selections),
                            Box::new(FieldProjectionCondition::SkipIfVariable(skip_if.clone())),
                        );
                    }

                    let mut condition_for_field: FieldProjectionCondition = condition.clone();
                    condition_for_field = FieldProjectionCondition::And(
                        Box::new(condition_for_field),
                        Box::new(FieldProjectionCondition::FieldTypeCondition(
                            possible_types_for_field,
                        )),
                    );
                    if let Some(include_if) = &field.include_if {
                        condition_for_field = FieldProjectionCondition::And(
                            Box::new(condition_for_field),
                            Box::new(FieldProjectionCondition::IncludeIfVariable(
                                include_if.clone(),
                            )),
                        );
                    }
                    if let Some(skip_if) = &field.skip_if {
                        condition_for_field = FieldProjectionCondition::And(
                            Box::new(condition_for_field),
                            Box::new(FieldProjectionCondition::SkipIfVariable(skip_if.clone())),
                        );
                    }
                    if let Some(enum_values) = schema_metadata.enum_values.get(field_type) {
                        condition_for_field = FieldProjectionCondition::And(
                            Box::new(condition_for_field),
                            Box::new(FieldProjectionCondition::EnumValuesCondition(
                                enum_values.clone(),
                            )),
                        );
                    }

                    if let Some(existing_field) = field_selections.get_mut(response_key) {
                        existing_field.conditions = FieldProjectionCondition::Or(
                            Box::new(existing_field.conditions.clone()),
                            Box::new(condition_for_field),
                        );

                        if let Some(new_selections) = {
                            FieldProjectionPlan::from_selection_set(
                                &field.selections,
                                schema_metadata,
                                field_type,
                                &conditions_for_selections,
                            )
                        } {
                            match existing_field.selections {
                                Some(ref mut selections) => {
                                    selections.extend(new_selections);
                                }
                                None => {
                                    existing_field.selections = Some(new_selections);
                                }
                            }
                        }
                    } else {
                        let new_plan = FieldProjectionPlan {
                            field_name: field_name.to_string(),
                            field_type: field_type.to_string(),
                            response_key: response_key.to_string(),
                            conditions: condition_for_field,
                            selections: FieldProjectionPlan::from_selection_set(
                                &field.selections,
                                schema_metadata,
                                field_type,
                                &conditions_for_selections,
                            ),
                        };
                        field_selections.insert(response_key.to_string(), new_plan);
                    }
                }
                SelectionItem::InlineFragment(inline_fragment) => {
                    let inline_fragment_type = &inline_fragment.type_condition;
                    let mut condition_for_inline_fragment = condition.clone();
                    let possible_types_for_inline_fragment = schema_metadata
                        .possible_types
                        .get_possible_types(inline_fragment_type);
                    condition_for_inline_fragment = FieldProjectionCondition::And(
                        Box::new(condition_for_inline_fragment),
                        Box::new(FieldProjectionCondition::ParentTypeCondition(
                            possible_types_for_inline_fragment,
                        )),
                    );
                    if let Some(skip_if) = &inline_fragment.skip_if {
                        condition_for_inline_fragment = FieldProjectionCondition::And(
                            Box::new(condition_for_inline_fragment),
                            Box::new(FieldProjectionCondition::SkipIfVariable(skip_if.clone())),
                        );
                    }
                    if let Some(include_if) = &inline_fragment.include_if {
                        condition_for_inline_fragment = FieldProjectionCondition::And(
                            Box::new(condition_for_inline_fragment),
                            Box::new(FieldProjectionCondition::IncludeIfVariable(
                                include_if.clone(),
                            )),
                        );
                    }
                    if let Some(inline_fragment_selections) =
                        FieldProjectionPlan::from_selection_set(
                            &inline_fragment.selections,
                            schema_metadata,
                            inline_fragment_type,
                            &condition_for_inline_fragment,
                        )
                    {
                        for selection in inline_fragment_selections {
                            if let Some(existing_field) =
                                field_selections.get_mut(&selection.response_key)
                            {
                                existing_field.conditions = FieldProjectionCondition::Or(
                                    Box::new(existing_field.conditions.clone()),
                                    Box::new(selection.conditions),
                                );
                                if let Some(new_selections) = selection.selections {
                                    match existing_field.selections {
                                        Some(ref mut selections) => {
                                            selections.extend(new_selections);
                                        }
                                        None => {
                                            existing_field.selections = Some(new_selections);
                                        }
                                    }
                                }
                            } else {
                                field_selections
                                    .insert(selection.response_key.to_string(), selection);
                            }
                        }
                    }
                }
                SelectionItem::FragmentSpread(_name_ref) => {
                    // Fragment spreads should not exist in the final response projection.
                    unreachable!(
                        "Fragment spreads should not exist in the final response projection."
                    );
                }
            }
        }

        if field_selections.is_empty() {
            None
        } else {
            Some(
                field_selections
                    .into_iter()
                    .map(|(_, selection)| selection)
                    .collect::<Vec<_>>(),
            )
        }
    }
    pub fn from_operation(
        operation: &OperationDefinition,
        schema_metadata: &SchemaMetadata,
    ) -> (&'static str, Vec<FieldProjectionPlan>) {
        let root_type_name = match operation.operation_kind {
            Some(OperationKind::Query) => "Query",
            Some(OperationKind::Mutation) => "Mutation",
            Some(OperationKind::Subscription) => "Subscription",
            None => "Query",
        };

        let field_type_conditions = schema_metadata
            .possible_types
            .get_possible_types(root_type_name);
        let conditions = FieldProjectionCondition::ParentTypeCondition(field_type_conditions);
        (
            root_type_name,
            FieldProjectionPlan::from_selection_set(
                &operation.selection_set,
                schema_metadata,
                root_type_name,
                &conditions,
            )
            .unwrap_or_default(),
        )
    }
}

#[instrument(level = "trace", skip_all)]
pub fn project_by_operation(
    data: &Value,
    errors: &mut Vec<GraphQLError>,
    extensions: &HashMap<String, serde_json::Value>,
    operation_type_name: &str,
    selections: &Vec<FieldProjectionPlan>,
    variable_values: &Option<HashMap<String, serde_json::Value>>,
) -> Vec<u8> {
    // We may want to remove it, but let's see.
    let mut buffer: Vec<u8> = Vec::with_capacity(4096);

    buffer.write_all(b"{\"data\":").unwrap();

    if let Some(data_map) = data.as_object() {
        let mut first = true;
        project_selection_set_with_map(
            data_map,
            errors,
            selections,
            variable_values,
            operation_type_name,
            &mut buffer,
            &mut first, // Start with first as true to add the opening brace
        );
        if !first {
            buffer.push(b'}');
        } else {
            // If no selections were made, we should return an empty object
            buffer.write_all(b"{}").unwrap();
        }
    }

    if !errors.is_empty() {
        write!(
            buffer,
            ",\"errors\":{}",
            serde_json::to_string(&errors).unwrap()
        )
        .unwrap();
    }
    if !extensions.is_empty() {
        write!(
            buffer,
            ",\"extensions\":{}",
            serde_json::to_string(&extensions).unwrap()
        )
        .unwrap();
    }

    buffer.push(b'}');
    buffer
}

fn project_selection_set(
    data: &Value,
    errors: &mut Vec<GraphQLError>,
    selection: &FieldProjectionPlan,
    variable_values: &Option<HashMap<String, serde_json::Value>>,
    buffer: &mut Vec<u8>,
) -> () {
    match data {
        Value::Null => buffer.write(b"null").unwrap(),
        Value::Bool(true) => buffer.write(b"true").unwrap(),
        Value::Bool(false) => buffer.write(b"false").unwrap(),
        Value::U64(num) => buffer.write(num.to_string().as_bytes()).unwrap(),
        Value::I64(num) => buffer.write(num.to_string().as_bytes()).unwrap(),
        Value::F64(num) => buffer.write(num.to_string().as_bytes()).unwrap(),
        Value::String(value) => write_and_escape_string_writer(buffer, value).unwrap(),
        Value::Array(arr) => {
            buffer.push(b'[');
            let mut first = true;
            for item in arr.iter() {
                if !first {
                    buffer.push(b',');
                }
                project_selection_set(item, errors, selection, variable_values, buffer);
                first = false;
            }
            buffer.push(b']');
            0
        }
        Value::Object(obj) => {
            match selection.selections.as_ref() {
                Some(selections) => {
                    let mut first = true;
                    let type_name = obj
                        .iter()
                        .find(|(k, _)| k == &TYPENAME_FIELD)
                        .and_then(|(_, val)| val.as_str())
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
                        buffer.push(b'}');
                    } else {
                        // If no selections were made, we should return an empty object
                        buffer.write(b"{}").unwrap();
                    }
                    0
                }
                None => {
                    // If the selection set is not projected, we should return null
                    buffer.write(b"null").unwrap();
                    0
                }
            }
        }
    };
}

// TODO: simplfy args
#[allow(clippy::too_many_arguments)]
fn project_selection_set_with_map(
    obj: &Vec<(&str, Value)>,
    errors: &mut Vec<GraphQLError>,
    selections: &Vec<FieldProjectionPlan>,
    variable_values: &Option<HashMap<String, serde_json::Value>>,
    parent_type_name: &str,
    buffer: &mut Vec<u8>,
    first: &mut bool,
) {
    for selection in selections {
        let field_val = obj
            .iter()
            .find_map(|(key, val)| {
                if key == &selection.field_name {
                    Some(val)
                } else {
                    None
                }
            })
            .or_else(|| {
                obj.iter().find_map(|(key, val)| {
                    if key == &selection.response_key {
                        Some(val)
                    } else {
                        None
                    }
                })
            });
        match selection.conditions.check(
            parent_type_name,
            &selection.field_type,
            &field_val,
            variable_values,
        ) {
            Ok(_) => {
                if *first {
                    buffer.push(b'{');
                } else {
                    buffer.push(b',');
                }
                *first = false;

                buffer.push(b'"');
                buffer.write(selection.response_key.as_bytes()).unwrap();
                buffer.write(b"\":").unwrap();

                if let Some(field_val) = field_val {
                    project_selection_set(field_val, errors, selection, variable_values, buffer);
                } else if selection.field_name == TYPENAME_FIELD {
                    // If the field is TYPENAME_FIELD, we should set it to the parent type name
                    buffer.push(b'"');
                    buffer.write(parent_type_name.as_bytes()).unwrap();
                    buffer.push(b'"');
                } else {
                    // If the field is not found in the object, set it to Null
                    buffer.write(b"null").unwrap();
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
                    buffer.push(b'{');
                } else {
                    buffer.push(b',');
                }
                *first = false;

                buffer.push(b'"');
                buffer.write(selection.response_key.as_bytes()).unwrap();
                buffer.write(b"\":null").unwrap();
                errors.push(GraphQLError {
                    message: "Value is not a valid enum value".to_string(),
                    locations: None,
                    path: None,
                    extensions: None,
                });
            }
            Err(FieldProjectionConditionError::InvalidFieldType) => {
                if *first {
                    buffer.push(b'{');
                } else {
                    buffer.push(b',');
                }
                *first = false;

                // Skip this field as the field type does not match
                buffer.push(b'"');
                buffer.write(selection.response_key.as_bytes()).unwrap();
                buffer.write(b"\":null").unwrap();
            }
        }
    }
}
