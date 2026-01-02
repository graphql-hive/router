use ahash::HashSet;
use indexmap::IndexMap;
use std::fmt::{Display, Formatter as FmtFormatter, Result as FmtResult};
use std::sync::Arc;
use tracing::warn;

use hive_router_query_planner::{
    ast::{
        operation::OperationDefinition,
        selection_item::SelectionItem,
        selection_set::{FieldSelection, InlineFragmentSelection, SelectionSet},
    },
    state::supergraph_state::OperationKind,
    utils::pretty_display::{get_indent, PrettyDisplay},
};

use crate::{introspection::schema::SchemaMetadata, utils::consts::TYPENAME_FIELD_NAME};

#[derive(Debug, Clone)]
pub enum TypeCondition {
    Exact(String),
    OneOf(HashSet<String>),
}

impl TypeCondition {
    pub fn union(self, other: TypeCondition) -> TypeCondition {
        use TypeCondition::*;
        match (self, other) {
            (Exact(left), Exact(right)) => {
                if left == right {
                    // Or(Exact(A), Exact(A)) -> Exact(A)
                    Exact(left)
                } else {
                    // Or(Exact(A), Exact(B)) -> OneOf(A, B)
                    OneOf(HashSet::from_iter(vec![left, right]))
                }
            }
            (OneOf(mut types), Exact(exact)) | (Exact(exact), OneOf(mut types)) => {
                // Or(OneOf(A, B), Exact(C)) -> OneOf(A, B, C)
                // Or(Exact(A), OneOf(A, B)) -> OneOf(A, B)
                types.insert(exact);
                OneOf(types)
            }
            (OneOf(mut left), OneOf(right)) => {
                // Or(OneOf(A, B), OneOf(C, D)) -> OneOf(A, B, C, D)
                left.extend(right);
                OneOf(left)
            }
        }
    }

    pub fn intersect(self, other: TypeCondition) -> TypeCondition {
        use TypeCondition::*;
        match (self, other) {
            (Exact(left), Exact(right)) => {
                if left == right {
                    // And(Exact(A), Exact(A)) -> Exact(A)
                    Exact(left)
                } else {
                    // And(Exact(A), Exact(B)) is impossible,
                    // so we return an empty OneOf
                    // to represent a condition that can never be true.
                    OneOf(HashSet::default())
                }
            }
            (OneOf(types), Exact(exact)) | (Exact(exact), OneOf(types)) => {
                if types.contains(&exact) {
                    // And(OneOf(A, B), Exact(A)) -> Exact(A)
                    Exact(exact)
                } else {
                    // And(Exact(A), OneOf(B, C)) is impossible,
                    // so we return an empty OneOf
                    // to represent a condition that can never be true.
                    OneOf(HashSet::default())
                }
            }
            (OneOf(mut left), OneOf(right)) => {
                left.retain(|t| right.contains(t));
                if left.len() == 1 {
                    // And(OneOf(A, B), OneOf(A, C)) -> Exact(A)
                    Exact(left.into_iter().next().expect("Set has one element"))
                } else {
                    // And(OneOf(A, B, C), OneOf(B, C)) -> OneOf(B, C)
                    // And(OneOf(A, B), OneOf(C, D)) -> OneOf()
                    OneOf(left)
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
pub enum ProjectionValueSource {
    /// Represents the entire response data from subgraphs.
    ResponseData {
        selections: Option<Arc<Vec<FieldProjectionPlan>>>,
    },
    /// Represents a null value.
    Null,
}

#[derive(Debug, Clone)]
pub struct FieldProjectionPlan {
    pub field_name: String,
    pub field_type: String,
    pub response_key: String,
    pub conditions: FieldProjectionCondition,
    pub value: ProjectionValueSource,
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

impl FieldProjectionCondition {
    /// Combines two conditions with AND logic, reducing them to their minimum form
    pub fn and(&self, right: FieldProjectionCondition) -> FieldProjectionCondition {
        use FieldProjectionCondition::*;

        match (self, right) {
            (ParentTypeCondition(left), ParentTypeCondition(right)) => {
                ParentTypeCondition(left.clone().intersect(right))
            }
            (FieldTypeCondition(left), FieldTypeCondition(right)) => {
                FieldTypeCondition(left.clone().intersect(right))
            }
            (EnumValuesCondition(left), EnumValuesCondition(right)) => {
                let mut left = left.clone();
                left.retain(|v| right.contains(v));
                EnumValuesCondition(left)
            }
            (left, right) => And(Box::new(left.clone()), Box::new(right)),
        }
    }

    /// Combines two conditions with OR logic, reducing them to their minimum form
    pub fn or(&self, right: FieldProjectionCondition) -> FieldProjectionCondition {
        use FieldProjectionCondition::*;

        match (self, right) {
            (ParentTypeCondition(left), ParentTypeCondition(right)) => {
                ParentTypeCondition(left.clone().union(right))
            }
            (FieldTypeCondition(left), FieldTypeCondition(right)) => {
                FieldTypeCondition(left.clone().union(right))
            }
            (EnumValuesCondition(left), EnumValuesCondition(right)) => {
                let mut result = left.clone();
                result.extend(right);
                EnumValuesCondition(result)
            }
            (left, right) => Or(Box::new(left.clone()), Box::new(right)),
        }
    }
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
            condition = condition.and(FieldProjectionCondition::IncludeIfVariable(
                include_if_var.clone(),
            ));
        }
        if let Some(skip_if_var) = skip_if {
            condition = condition.and(FieldProjectionCondition::SkipIfVariable(
                skip_if_var.clone(),
            ));
        }
        condition
    }

    fn merge_plan(
        field_selections: &mut IndexMap<String, FieldProjectionPlan>,
        plan_to_merge: FieldProjectionPlan,
    ) {
        if let Some(existing_plan) = field_selections.get_mut(&plan_to_merge.response_key) {
            existing_plan.conditions = existing_plan.conditions.or(plan_to_merge.conditions);

            match (&mut existing_plan.value, plan_to_merge.value) {
                (
                    ProjectionValueSource::ResponseData {
                        selections: existing_selections,
                    },
                    ProjectionValueSource::ResponseData {
                        selections: new_selections,
                    },
                ) => {
                    if let Some(new_selections) = new_selections {
                        match existing_selections {
                            Some(selections) => {
                                Arc::make_mut(selections).extend(
                                    Arc::try_unwrap(new_selections)
                                        .unwrap_or_else(|arc| (*arc).clone()),
                                );
                            }
                            None => *existing_selections = Some(new_selections),
                        }
                    }
                }
                (ProjectionValueSource::Null, ProjectionValueSource::Null) => {
                    // Both plans have `Null` value source, so nothing to merge
                }
                _ => {
                    // This case should not be reached during initial plan construction,
                    // as `Null` is only introduced during the authorization step.
                    // If we merge a plan, it's always to combine selections.
                    warn!("Merging plans with `Null` value source is not supported during initial plan construction.");
                    existing_plan.value = ProjectionValueSource::Null;
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
                Some(f) => f.output_type_name.clone(),
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

        let mut condition_for_field =
            parent_condition.and(FieldProjectionCondition::FieldTypeCondition(type_condition));
        condition_for_field = Self::apply_directive_conditions(
            condition_for_field,
            &field.include_if,
            &field.skip_if,
        );

        if let Some(enum_values) = schema_metadata.enum_values.get(&field_type) {
            condition_for_field = condition_for_field.and(
                FieldProjectionCondition::EnumValuesCondition(enum_values.clone()),
            );
        }

        let new_plan = FieldProjectionPlan {
            field_name: field_name.to_string(),
            field_type: field_type.clone(),
            response_key,
            conditions: condition_for_field,
            value: ProjectionValueSource::ResponseData {
                selections: Self::from_selection_set(
                    &field.selections,
                    schema_metadata,
                    &field_type,
                    &conditions_for_selections,
                )
                .map(Arc::new),
            },
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

        let mut condition_for_fragment = parent_condition.and(
            FieldProjectionCondition::ParentTypeCondition(type_condition),
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

    pub fn with_new_value(&self, new_value: ProjectionValueSource) -> FieldProjectionPlan {
        FieldProjectionPlan {
            field_name: self.field_name.clone(),
            field_type: self.field_type.clone(),
            response_key: self.response_key.clone(),
            conditions: self.conditions.clone(),
            value: new_value,
        }
    }
}

impl Display for TypeCondition {
    fn fmt(&self, f: &mut FmtFormatter<'_>) -> FmtResult {
        match self {
            TypeCondition::Exact(type_name) => write!(f, "Exact({})", type_name),
            TypeCondition::OneOf(types) => {
                write!(f, "OneOf(")?;
                let types_vec: Vec<_> = types.iter().collect();
                for (i, type_name) in types_vec.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", type_name)?;
                }
                write!(f, ")")
            }
        }
    }
}

impl Display for FieldProjectionCondition {
    fn fmt(&self, f: &mut FmtFormatter<'_>) -> FmtResult {
        match self {
            FieldProjectionCondition::IncludeIfVariable(var) => {
                write!(f, "Include(if: ${})", var)
            }
            FieldProjectionCondition::SkipIfVariable(var) => {
                write!(f, "Skip(if: ${})", var)
            }
            FieldProjectionCondition::ParentTypeCondition(tc) => {
                write!(f, "ParentType({})", tc)
            }
            FieldProjectionCondition::FieldTypeCondition(tc) => {
                write!(f, "FieldType({})", tc)
            }
            FieldProjectionCondition::EnumValuesCondition(values) => {
                write!(f, "EnumValues(")?;
                let values_vec: Vec<_> = values.iter().collect();
                for (i, value) in values_vec.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", value)?;
                }
                write!(f, ")")
            }
            FieldProjectionCondition::Or(left, right) => {
                write!(f, "({} OR {})", left, right)
            }
            FieldProjectionCondition::And(left, right) => {
                write!(f, "({} AND {})", left, right)
            }
        }
    }
}

impl Display for FieldProjectionPlan {
    fn fmt(&self, f: &mut FmtFormatter<'_>) -> FmtResult {
        self.pretty_fmt(f, 0)
    }
}

impl PrettyDisplay for FieldProjectionPlan {
    fn pretty_fmt(&self, f: &mut FmtFormatter<'_>, depth: usize) -> FmtResult {
        let indent = get_indent(depth);

        if self.response_key == self.field_name {
            writeln!(f, "{}{}: {} {{", indent, self.response_key, self.field_type)?;
        } else {
            writeln!(
                f,
                "{}{} (alias for {}): {} {{",
                indent, self.response_key, self.field_name, self.field_type
            )?;
        }

        writeln!(f, "{}  conditions: {}", indent, self.conditions)?;

        match &self.value {
            ProjectionValueSource::ResponseData { selections } => {
                if let Some(selections) = selections {
                    writeln!(f, "{}  selections:", indent)?;
                    for selection in selections.iter() {
                        selection.pretty_fmt(f, depth + 2)?;
                    }
                }
            }
            ProjectionValueSource::Null => {
                writeln!(f, "{}  value: Null", indent)?;
            }
        }

        writeln!(f, "{}}}", indent)?;
        Ok(())
    }
}
