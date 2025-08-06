use std::collections::HashMap;

use indexmap::IndexMap;
use query_planner::{
    ast::{
        operation::OperationDefinition, selection_item::SelectionItem, selection_set::SelectionSet,
    },
    state::supergraph_state::OperationKind,
};
use tracing::warn;

use crate::{
    response::value::Value, schema::metadata::SchemaMetadata, utils::consts::TYPENAME_FIELD_NAME,
};

pub struct ProjectionPlan<'a> {
    pub root_selections: Vec<FieldProjectionPlan<'a>>,
}

impl<'a> ProjectionPlan<'a> {
    pub fn from_operation(
        operation: &'a OperationDefinition,
        schema_metadata: &'a SchemaMetadata,
    ) -> (&'static str, ProjectionPlan<'a>) {
        let root_type_name = match operation.operation_kind {
            Some(OperationKind::Query) => "Query",
            Some(OperationKind::Mutation) => "Mutation",
            Some(OperationKind::Subscription) => "Subscription",
            None => "Query",
        };

        let field_type_conditions = schema_metadata
            .possible_types
            .get_possible_types_sorted(root_type_name);
        let conditions = FieldProjectionCondition::ParentTypeCondition(field_type_conditions);
        let plan = ProjectionPlan {
            root_selections: FieldProjectionPlan::from_selection_set(
                &operation.selection_set,
                schema_metadata,
                root_type_name,
                conditions,
            )
            .unwrap_or_default(),
        };

        (root_type_name, plan)
    }
}

#[derive(Debug)]
pub struct FieldProjectionPlan<'a> {
    pub field_name: &'a str,
    pub field_type: &'a str,
    pub response_key: &'a str,
    pub conditions: FieldProjectionCondition<'a>,
    pub selections: Option<Vec<FieldProjectionPlan<'a>>>,
}

#[derive(Debug, Clone)]
pub enum FieldProjectionCondition<'a> {
    IncludeIfVariable(&'a str),
    SkipIfVariable(&'a str),
    ParentTypeCondition(Vec<&'a str>),
    FieldTypeCondition(Vec<&'a str>),
    EnumValuesCondition(Vec<&'a str>),
    Or(
        Box<FieldProjectionCondition<'a>>,
        Box<FieldProjectionCondition<'a>>,
    ),
    And(
        Box<FieldProjectionCondition<'a>>,
        Box<FieldProjectionCondition<'a>>,
    ),
}

pub enum FieldProjectionConditionError {
    InvalidParentType,
    InvalidFieldType,
    Skip,
    InvalidEnumValue,
}

impl<'a> FieldProjectionCondition<'a> {
    pub fn check(
        &self,
        parent_type_name: &str,
        field_type_name: &str,
        field_value: Option<&Value>,
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
                        .get(*variable_name)
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
                        .get(*variable_name)
                        .is_some_and(|v| v.as_bool().unwrap_or(false))
                    {
                        return Err(FieldProjectionConditionError::Skip);
                    }
                }
                Ok(())
            }
            FieldProjectionCondition::ParentTypeCondition(possible_types) => {
                if possible_types.binary_search(&parent_type_name).is_ok() {
                    Ok(())
                } else {
                    Err(FieldProjectionConditionError::InvalidParentType)
                }
            }
            FieldProjectionCondition::FieldTypeCondition(possible_types) => {
                let field_type_name = field_value
                    .and_then(|value| value.as_object())
                    .and_then(|obj| {
                        obj.binary_search_by_key(&TYPENAME_FIELD_NAME, |(k, _)| *k)
                            .ok()
                            .and_then(|idx| obj[idx].1.as_str())
                    })
                    .unwrap_or(field_type_name);
                if possible_types.binary_search(&field_type_name).is_ok() {
                    Ok(())
                } else {
                    Err(FieldProjectionConditionError::InvalidFieldType)
                }
            }
            FieldProjectionCondition::EnumValuesCondition(enum_values) => {
                if let Some(Value::String(string_value)) = field_value {
                    if enum_values.binary_search(&string_value).is_ok() {
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

impl<'a> FieldProjectionPlan<'a> {
    pub fn from_selection_set(
        selection_set: &'a SelectionSet,
        schema_metadata: &'a SchemaMetadata,
        type_name: &'a str,
        condition: FieldProjectionCondition<'a>,
    ) -> Option<Vec<FieldProjectionPlan<'a>>> {
        let mut field_selections: IndexMap<String, FieldProjectionPlan> = IndexMap::new();
        for selection_item in &selection_set.items {
            match selection_item {
                SelectionItem::Field(field) => {
                    let field_name = field.name.as_str();
                    let response_key = field
                        .alias
                        .as_ref()
                        .map(|s| s.as_str())
                        .unwrap_or(field_name);
                    let field_type = if field_name == TYPENAME_FIELD_NAME {
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
                        .get_possible_types_sorted(field_type);

                    let mut conditions_for_selections =
                        FieldProjectionCondition::ParentTypeCondition(
                            possible_types_for_field.clone(),
                        );
                    if let Some(include_if) = &field.include_if {
                        conditions_for_selections = FieldProjectionCondition::And(
                            Box::new(conditions_for_selections),
                            Box::new(FieldProjectionCondition::IncludeIfVariable(include_if)),
                        );
                    }
                    if let Some(skip_if) = &field.skip_if {
                        conditions_for_selections = FieldProjectionCondition::And(
                            Box::new(conditions_for_selections),
                            Box::new(FieldProjectionCondition::SkipIfVariable(skip_if)),
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
                            Box::new(FieldProjectionCondition::IncludeIfVariable(include_if)),
                        );
                    }
                    if let Some(skip_if) = &field.skip_if {
                        condition_for_field = FieldProjectionCondition::And(
                            Box::new(condition_for_field),
                            Box::new(FieldProjectionCondition::SkipIfVariable(skip_if)),
                        );
                    }
                    // if let Some(enum_values) = schema_metadata.enum_values.get(field_type) {
                    //     condition_for_field = FieldProjectionCondition::And(
                    //         Box::new(condition_for_field),
                    //         Box::new(FieldProjectionCondition::EnumValuesCondition(
                    //             enum_values.clone(),
                    //         )),
                    //     );
                    // }

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
                                conditions_for_selections,
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
                            field_name: field_name,
                            field_type: field_type,
                            response_key: response_key,
                            conditions: condition_for_field,
                            selections: FieldProjectionPlan::from_selection_set(
                                &field.selections,
                                schema_metadata,
                                field_type,
                                conditions_for_selections,
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
                        .get_possible_types_sorted(inline_fragment_type);

                    condition_for_inline_fragment = FieldProjectionCondition::And(
                        Box::new(condition_for_inline_fragment),
                        Box::new(FieldProjectionCondition::ParentTypeCondition(
                            possible_types_for_inline_fragment,
                        )),
                    );
                    if let Some(skip_if) = &inline_fragment.skip_if {
                        condition_for_inline_fragment = FieldProjectionCondition::And(
                            Box::new(condition_for_inline_fragment),
                            Box::new(FieldProjectionCondition::SkipIfVariable(skip_if)),
                        );
                    }
                    if let Some(include_if) = &inline_fragment.include_if {
                        condition_for_inline_fragment = FieldProjectionCondition::And(
                            Box::new(condition_for_inline_fragment),
                            Box::new(FieldProjectionCondition::IncludeIfVariable(include_if)),
                        );
                    }
                    if let Some(inline_fragment_selections) =
                        FieldProjectionPlan::from_selection_set(
                            &inline_fragment.selections,
                            schema_metadata,
                            inline_fragment_type,
                            condition_for_inline_fragment,
                        )
                    {
                        for selection in inline_fragment_selections {
                            if let Some(existing_field) =
                                field_selections.get_mut(selection.response_key)
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
}
