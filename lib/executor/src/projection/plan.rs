use indexmap::IndexMap;
use std::collections::HashSet;
use tracing::warn;

use query_planner::{
    ast::{
        operation::OperationDefinition, selection_item::SelectionItem, selection_set::SelectionSet,
    },
    state::supergraph_state::OperationKind,
};

use crate::{introspection::schema::SchemaMetadata, utils::consts::TYPENAME_FIELD_NAME};

#[derive(Debug)]
pub struct FieldProjectionPlan {
    pub field_name: String,
    pub field_type: String,
    pub response_key: String,
    pub conditions: FieldProjectionCondition,
    pub selections: Option<Vec<FieldProjectionPlan>>,
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

impl FieldProjectionPlan {
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

    fn from_selection_set(
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
}
