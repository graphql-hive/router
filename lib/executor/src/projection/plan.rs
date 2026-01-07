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

use crate::projection::error::ProjectionError;
use crate::{introspection::schema::SchemaMetadata, utils::consts::TYPENAME_FIELD_NAME};

#[derive(Debug, Clone)]
pub enum TypeCondition {
    Exact(String),
    OneOf(HashSet<String>),
}

impl TypeCondition {
    pub fn matches(&self, type_name: &str) -> bool {
        match self {
            TypeCondition::Exact(expected) => type_name == expected,
            TypeCondition::OneOf(possible) => possible.contains(type_name),
        }
    }

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
    pub response_key: String,
    pub is_typename: bool,
    /// A condition that checks the name of the parent object.
    /// This is used to ensure that fields inside a fragment (e.g., `... on User`)
    /// are only applied when the parent object's type matches the fragment's type condition.
    /// If `None`, the plan applies to any parent type.
    pub parent_type_guard: Option<TypeCondition>,
    pub conditions: Option<FieldProjectionCondition>,
    pub value: ProjectionValueSource,
}

#[derive(Debug, Clone)]
pub enum FieldProjectionCondition {
    IncludeIfVariable(String),
    SkipIfVariable(String),
    ParentTypeCondition(TypeCondition),
    True,
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
    Fatal(ProjectionError),
}

impl From<ProjectionError> for FieldProjectionConditionError {
    fn from(err: ProjectionError) -> Self {
        FieldProjectionConditionError::Fatal(err)
    }
}

impl FieldProjectionCondition {
    /// Combines two conditions with AND logic, reducing them to their minimum form
    pub fn and(&self, right: FieldProjectionCondition) -> FieldProjectionCondition {
        use FieldProjectionCondition::*;

        match (self, right) {
            (True, other) => other.clone(),
            (other, True) => other.clone(),
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
            (True, _) | (_, True) => True,
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

        let conditions = FieldProjectionCondition::True;
        let mut plans = Self::from_selection_set(
            &operation.selection_set,
            schema_metadata,
            root_type_name,
            &conditions,
        )
        .unwrap_or_default();

        for plan in &mut plans {
            Self::remove_redundant_child_guards(plan, schema_metadata);
        }

        (root_type_name, plans)
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

    /// Extracts type names from a TypeCondition for easier comparison
    fn type_names_from(condition: &TypeCondition) -> Vec<&str> {
        match condition {
            TypeCondition::Exact(ty) => vec![ty.as_str()],
            TypeCondition::OneOf(types) => types.iter().map(|s| s.as_str()).collect(),
        }
    }

    /// Recursively removes redundant type guards from child selections.
    ///
    /// A type guard is redundant when it covers exactly all possible types that can appear
    /// in that position. For example, if a field `children` can only return types A or B
    /// (based on the parent's type guard), and a child selection has guard `OneOf(A, B)`,
    /// then that guard is redundant since it will always match.
    ///
    /// This optimization must run after all fragment merging is complete to correctly
    /// determine the full set of possible types.
    fn remove_redundant_child_guards(
        parent_field: &mut FieldProjectionPlan,
        schema_metadata: &SchemaMetadata,
    ) {
        let ProjectionValueSource::ResponseData { selections } = &mut parent_field.value else {
            return;
        };

        let Some(selections_arc) = selections else {
            return;
        };

        let selections_mut = Arc::make_mut(selections_arc);

        // Compute possible child types only if parent has a type guard
        let possible_child_types = parent_field.parent_type_guard.as_ref().map(|parent_guard| {
            let parent_types = Self::type_names_from(parent_guard);

            // For each parent type, lookup what type this field returns
            let mut types = std::collections::HashSet::new();
            for parent_type in parent_types {
                if let Some(field_info) = schema_metadata
                    .type_fields
                    .get(parent_type)
                    .and_then(|fields| fields.get(&parent_field.field_name))
                {
                    types.insert(field_info.output_type_name.as_str());
                }
            }
            types
        });

        for child in selections_mut {
            // Remove redundant guard if this child's guard covers all possible types
            if let (Some(child_guard), Some(possible_types)) =
                (&child.parent_type_guard, &possible_child_types)
            {
                let guard_types = Self::type_names_from(child_guard);
                let guard_set: std::collections::HashSet<_> = guard_types.into_iter().collect();

                if &guard_set == possible_types {
                    child.parent_type_guard = None;
                }
            }

            Self::remove_redundant_child_guards(child, schema_metadata);
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
        let Some(existing_plan) = field_selections.get_mut(&plan_to_merge.response_key) else {
            field_selections.insert(plan_to_merge.response_key.clone(), plan_to_merge);
            return;
        };

        // Merge type guards with OR semantics
        existing_plan.parent_type_guard = match (
            existing_plan.parent_type_guard.take(),
            plan_to_merge.parent_type_guard,
        ) {
            (Some(left), Some(right)) => Some(left.union(right)),
            (Some(guard), None) | (None, Some(guard)) => Some(guard),
            (None, None) => None,
        };

        existing_plan.conditions = match (existing_plan.conditions.take(), plan_to_merge.conditions)
        {
            (Some(left), Some(right)) => Some(left.or(right)),
            (Some(cond), None) | (None, Some(cond)) => Some(cond),
            (None, None) => None,
        };

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
                            // Recursively merge child selections instead of just extending
                            let selections_mut = Arc::make_mut(selections);
                            let new_selections_vec = Arc::try_unwrap(new_selections)
                                .unwrap_or_else(|arc| (*arc).clone());

                            // Convert existing selections to a map for efficient lookup
                            let mut selections_map: IndexMap<String, FieldProjectionPlan> =
                                selections_mut
                                    .drain(..)
                                    .map(|plan| (plan.response_key.clone(), plan))
                                    .collect();

                            // Merge each new selection
                            for new_plan in new_selections_vec {
                                Self::merge_plan(&mut selections_map, new_plan);
                            }

                            // Convert back to Vec
                            selections_mut.extend(selections_map.into_values());
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
    }

    /// Simplifies a condition by removing redundant parent type conditions.
    ///
    /// A condition is redundant if it matches the parent_type_guard (since the guard
    /// already ensures that constraint). Also removes bare `True` conditions which
    /// add no value.
    fn simplify_condition(
        condition: FieldProjectionCondition,
        parent_type_guard: &Option<TypeCondition>,
    ) -> Option<FieldProjectionCondition> {
        let Some(TypeCondition::Exact(guard_type)) = parent_type_guard else {
            return Some(condition);
        };

        match condition {
            // Remove bare True conditions
            FieldProjectionCondition::True => None,

            // Remove redundant ParentTypeCondition that matches the guard
            FieldProjectionCondition::ParentTypeCondition(TypeCondition::Exact(cond_type))
                if &cond_type == guard_type =>
            {
                None
            }

            // Recursively simplify And expressions
            FieldProjectionCondition::And(left, right) => {
                let left_simplified = Self::simplify_condition(*left, parent_type_guard);
                let right_simplified = Self::simplify_condition(*right, parent_type_guard);

                match (left_simplified, right_simplified) {
                    (None, None) => None,
                    (Some(cond), None) | (None, Some(cond)) => Some(cond),
                    (Some(l), Some(r)) => Some(l.and(r)),
                }
            }

            // Keep other conditions as-is
            other => Some(other),
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

        let parent_type_guard = Self::get_type_guard(parent_condition);
        let conditions_for_selections = Self::apply_directive_conditions(
            match &parent_type_guard {
                None => FieldProjectionCondition::True,
                _ => FieldProjectionCondition::ParentTypeCondition(type_condition.clone()),
            },
            &field.include_if,
            &field.skip_if,
        );

        let mut condition_for_field = if schema_metadata.is_union_type(&field_type)
            || schema_metadata.is_interface_type(&field_type)
        {
            parent_condition.and(FieldProjectionCondition::FieldTypeCondition(type_condition))
        } else {
            // It makes no sense to have a field type condition for concrete types
            // as they'd always evaluate to true.
            parent_condition.clone()
        };
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

        let final_conditions = Self::simplify_condition(condition_for_field, &parent_type_guard);

        let new_plan = FieldProjectionPlan {
            field_name: field_name.to_string(),
            response_key,
            parent_type_guard,
            is_typename: field_name == TYPENAME_FIELD_NAME,
            conditions: final_conditions,
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
            FieldProjectionCondition::ParentTypeCondition(type_condition.clone()),
        );

        condition_for_fragment = Self::apply_directive_conditions(
            condition_for_fragment,
            &inline_fragment.include_if,
            &inline_fragment.skip_if,
        );

        if let Some(mut inline_fragment_selections) = Self::from_selection_set(
            &inline_fragment.selections,
            schema_metadata,
            inline_fragment_type,
            &condition_for_fragment,
        ) {
            // Update the type guard for all selections from this fragment
            for selection in &mut inline_fragment_selections {
                selection.parent_type_guard = Some(type_condition.clone());
            }

            for selection in inline_fragment_selections {
                Self::merge_plan(field_selections, selection);
            }
        }
    }

    pub fn with_new_value(&self, new_value: ProjectionValueSource) -> FieldProjectionPlan {
        FieldProjectionPlan {
            field_name: self.field_name.clone(),
            response_key: self.response_key.clone(),
            parent_type_guard: self.parent_type_guard.clone(),
            conditions: self.conditions.clone(),
            is_typename: self.is_typename,
            value: new_value,
        }
    }

    fn get_type_guard(condition: &FieldProjectionCondition) -> Option<TypeCondition> {
        match condition {
            FieldProjectionCondition::True => None,
            FieldProjectionCondition::ParentTypeCondition(tc) => Some(tc.clone()),
            FieldProjectionCondition::And(a, b) => {
                Self::get_type_guard(a).or_else(|| Self::get_type_guard(b))
            }
            FieldProjectionCondition::Or(a, b) => {
                match (Self::get_type_guard(a), Self::get_type_guard(b)) {
                    (Some(ga), Some(gb)) => Some(ga.union(gb)),
                    (Some(guard), None) | (None, Some(guard)) => Some(guard),
                    (None, None) => None,
                }
            }
            _ => None,
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
            FieldProjectionCondition::True => write!(f, "True"),
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
            writeln!(f, "{}{}: {{", indent, self.response_key)?;
        } else {
            writeln!(
                f,
                "{}{} (alias for {}) {{",
                indent, self.response_key, self.field_name
            )?;
        }

        if let Some(parent_type_guard) = self.parent_type_guard.as_ref() {
            writeln!(f, "{}  type guard: {}", indent, parent_type_guard)?;
        }

        if let Some(conditions) = self.conditions.as_ref() {
            writeln!(f, "{}  conditions: {}", indent, conditions)?;
        }

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
