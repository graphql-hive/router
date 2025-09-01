use indexmap::IndexMap;
use std::collections::HashSet;
use tracing::warn;

use hive_router_query_planner::{
    ast::{
        operation::OperationDefinition,
        selection_item::SelectionItem,
        selection_set::{FieldSelection, InlineFragmentSelection, SelectionSet},
    },
    state::supergraph_state::OperationKind,
};

use crate::{introspection::schema::SchemaMetadata, utils::consts::TYPENAME_FIELD_NAME};

#[derive(Debug, Clone)]
pub enum TypeCondition {
    Exact(String),
    OneOf(HashSet<String>),
}

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
    ParentTypeCondition(TypeCondition),
    FieldTypeCondition(TypeCondition),
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

        let root_type_condition = if schema_metadata.is_object_type(root_type_name) {
            TypeCondition::Exact(root_type_name.to_string())
        } else {
            TypeCondition::OneOf(
                schema_metadata
                    .possible_types
                    .get_possible_types(root_type_name),
            )
        };

        let conditions = FieldProjectionCondition::ParentTypeCondition(root_type_condition);
        (
            root_type_name,
            Self::from_selection_set(
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
        parent_type_name: &str,
        parent_condition: &FieldProjectionCondition,
    ) -> Option<Vec<FieldProjectionPlan>> {
        let mut field_selections: IndexMap<String, FieldProjectionPlan> = IndexMap::new();

        for selection_item in &selection_set.items {
            match selection_item {
                SelectionItem::Field(field) => {
                    Self::process_field(
                        field,
                        &mut field_selections,
                        schema_metadata,
                        parent_type_name,
                        parent_condition,
                    );
                }
                SelectionItem::InlineFragment(inline_fragment) => {
                    Self::process_inline_fragment(
                        inline_fragment,
                        &mut field_selections,
                        schema_metadata,
                        parent_condition,
                    );
                }
                SelectionItem::FragmentSpread(_) => {
                    // Fragment spreads should have been inlined by this stage.
                    unreachable!(
                        "Fragment spreads should not exist in the final response projection."
                    );
                }
            }
        }

        if field_selections.is_empty() {
            None
        } else {
            Some(field_selections.into_values().collect())
        }
    }

    fn apply_directive_conditions(
        mut condition: FieldProjectionCondition,
        include_if: &Option<String>,
        skip_if: &Option<String>,
    ) -> FieldProjectionCondition {
        if let Some(include_if_var) = include_if {
            condition = FieldProjectionCondition::And(
                Box::new(condition),
                Box::new(FieldProjectionCondition::IncludeIfVariable(
                    include_if_var.clone(),
                )),
            );
        }
        if let Some(skip_if_var) = skip_if {
            condition = FieldProjectionCondition::And(
                Box::new(condition),
                Box::new(FieldProjectionCondition::SkipIfVariable(
                    skip_if_var.clone(),
                )),
            );
        }
        condition
    }

    fn merge_plan(
        field_selections: &mut IndexMap<String, FieldProjectionPlan>,
        plan_to_merge: FieldProjectionPlan,
    ) {
        if let Some(existing_plan) = field_selections.get_mut(&plan_to_merge.response_key) {
            existing_plan.conditions = FieldProjectionCondition::Or(
                Box::new(existing_plan.conditions.clone()),
                Box::new(plan_to_merge.conditions),
            );

            if let Some(new_selections) = plan_to_merge.selections {
                match &mut existing_plan.selections {
                    Some(selections) => selections.extend(new_selections),
                    None => existing_plan.selections = Some(new_selections),
                }
            }
        } else {
            field_selections.insert(plan_to_merge.response_key.clone(), plan_to_merge);
        }
    }

    fn process_field(
        field: &FieldSelection,
        field_selections: &mut IndexMap<String, FieldProjectionPlan>,
        schema_metadata: &SchemaMetadata,
        parent_type_name: &str,
        parent_condition: &FieldProjectionCondition,
    ) {
        let field_name = &field.name;
        let response_key = field.alias.as_ref().unwrap_or(field_name).clone();

        let field_type = if field_name == TYPENAME_FIELD_NAME {
            "String".to_string()
        } else {
            let field_map = match schema_metadata.type_fields.get(parent_type_name) {
                Some(fields) => fields,
                None => {
                    warn!(
                        "No fields found for type `{}` in schema metadata.",
                        parent_type_name
                    );
                    return;
                }
            };
            match field_map.get(field_name) {
                Some(ft) => ft.clone(),
                None => {
                    warn!(
                        "Field `{}` not found in type `{}` in schema metadata.",
                        field_name, parent_type_name
                    );
                    return;
                }
            }
        };

        let type_condition = if schema_metadata.is_object_type(&field_type)
            || schema_metadata.is_scalar_type(&field_type)
        {
            TypeCondition::Exact(field_type.to_string())
        } else {
            TypeCondition::OneOf(
                schema_metadata
                    .possible_types
                    .get_possible_types(&field_type),
            )
        };
        let conditions_for_selections = Self::apply_directive_conditions(
            FieldProjectionCondition::ParentTypeCondition(type_condition.clone()),
            &field.include_if,
            &field.skip_if,
        );

        let mut condition_for_field = FieldProjectionCondition::And(
            Box::new(parent_condition.clone()),
            Box::new(FieldProjectionCondition::FieldTypeCondition(type_condition)),
        );
        condition_for_field = Self::apply_directive_conditions(
            condition_for_field,
            &field.include_if,
            &field.skip_if,
        );

        if let Some(enum_values) = schema_metadata.enum_values.get(&field_type) {
            condition_for_field = FieldProjectionCondition::And(
                Box::new(condition_for_field),
                Box::new(FieldProjectionCondition::EnumValuesCondition(
                    enum_values.clone(),
                )),
            );
        }

        let new_plan = FieldProjectionPlan {
            field_name: field_name.to_string(),
            field_type: field_type.clone(),
            response_key,
            conditions: condition_for_field,
            selections: Self::from_selection_set(
                &field.selections,
                schema_metadata,
                &field_type,
                &conditions_for_selections,
            ),
        };

        Self::merge_plan(field_selections, new_plan);
    }

    fn process_inline_fragment(
        inline_fragment: &InlineFragmentSelection,
        field_selections: &mut IndexMap<String, FieldProjectionPlan>,
        schema_metadata: &SchemaMetadata,
        parent_condition: &FieldProjectionCondition,
    ) {
        let inline_fragment_type = &inline_fragment.type_condition;
        let type_condition = if schema_metadata.is_object_type(inline_fragment_type) {
            TypeCondition::Exact(inline_fragment_type.to_string())
        } else {
            TypeCondition::OneOf(
                schema_metadata
                    .possible_types
                    .get_possible_types(inline_fragment_type),
            )
        };

        let mut condition_for_fragment = FieldProjectionCondition::And(
            Box::new(parent_condition.clone()),
            Box::new(FieldProjectionCondition::ParentTypeCondition(
                type_condition,
            )),
        );

        condition_for_fragment = Self::apply_directive_conditions(
            condition_for_fragment,
            &inline_fragment.include_if,
            &inline_fragment.skip_if,
        );

        if let Some(inline_fragment_selections) = Self::from_selection_set(
            &inline_fragment.selections,
            schema_metadata,
            inline_fragment_type,
            &condition_for_fragment,
        ) {
            for selection in inline_fragment_selections {
                Self::merge_plan(field_selections, selection);
            }
        }
    }
}
