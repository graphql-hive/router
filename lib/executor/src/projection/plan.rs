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
use crate::{
    introspection::schema::{FieldNullability, SchemaMetadata},
    utils::consts::TYPENAME_FIELD_NAME,
};

#[derive(Debug, Clone, PartialEq, Eq)]
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
    pub nullability: FieldNullability,
    /// A condition that checks the name of the parent object.
    /// This is used to ensure that fields inside a fragment (e.g., `... on User`)
    /// are only applied when the parent object's type matches the fragment's type condition.
    /// If `None`, the plan applies to any parent type.
    pub parent_type_guard: Option<TypeCondition>,
    pub conditions: Option<FieldProjectionCondition>,
    pub value: ProjectionValueSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
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

    /// Combines two conditions with OR logic, reducing them to their minimum form.
    ///
    /// This method automatically deduplicates identical conditions to avoid creating
    /// redundant expressions like `X OR X`.
    pub fn or(&self, right: FieldProjectionCondition) -> FieldProjectionCondition {
        use FieldProjectionCondition::*;

        // Avoid creating duplicate OR expressions
        if self == &right {
            return self.clone();
        }

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

        let mut plans = Self::from_selection_set(
            &operation.selection_set,
            schema_metadata,
            root_type_name,
            &None,
        )
        .unwrap_or_default();

        let root_parent_types = HashSet::from_iter([root_type_name.to_string()]);
        for plan in &mut plans {
            Self::remove_redundant_child_guards(plan, schema_metadata, &root_parent_types);
        }

        (root_type_name, plans)
    }

    fn from_selection_set(
        selection_set: &SelectionSet,
        schema_metadata: &SchemaMetadata,
        parent_type_name: &str,
        parent_condition: &Option<FieldProjectionCondition>,
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

    fn possible_runtime_types(
        schema_metadata: &SchemaMetadata,
        type_name: &str,
    ) -> HashSet<String> {
        if schema_metadata.is_interface_type(type_name) || schema_metadata.is_union_type(type_name)
        {
            schema_metadata
                .get_possible_types(type_name)
                .cloned()
                .unwrap_or_default()
        } else {
            HashSet::from_iter([type_name.to_string()])
        }
    }

    fn possible_field_runtime_types(
        parent_field: &FieldProjectionPlan,
        schema_metadata: &SchemaMetadata,
        parent_type_names: &HashSet<String>,
    ) -> HashSet<String> {
        let mut field_runtime_types = HashSet::default();

        for parent_type_name in parent_type_names {
            let Some(field_info) = schema_metadata
                .type_fields
                .get(parent_type_name)
                .and_then(|fields| fields.get(&parent_field.field_name))
            else {
                continue;
            };

            field_runtime_types.extend(Self::possible_runtime_types(
                schema_metadata,
                &field_info.output_type_name,
            ));
        }

        field_runtime_types
    }

    fn guarded_parent_types(
        parent_type_names: &HashSet<String>,
        parent_type_guard: &Option<TypeCondition>,
    ) -> HashSet<String> {
        let Some(parent_type_guard) = parent_type_guard else {
            return parent_type_names.clone();
        };

        parent_type_names
            .iter()
            .filter(|type_name| parent_type_guard.matches(type_name))
            .cloned()
            .collect()
    }

    fn type_condition_covers_types(
        type_condition: &TypeCondition,
        type_names: &HashSet<String>,
    ) -> bool {
        !type_names.is_empty()
            && type_names
                .iter()
                .all(|type_name| type_condition.matches(type_name))
    }

    /// Recursively removes type guards that are always true at their response position.
    ///
    /// Every guard has a runtime cost during response projection: it resolves the parent
    /// `__typename` and checks the guard. This cleanup runs after fragment merging, when
    /// we know the full set of possible parent types for each selection-set position.
    /// Proper-subset guards created by splitting overlapping fragments are preserved.
    fn remove_redundant_child_guards(
        parent_field: &mut FieldProjectionPlan,
        schema_metadata: &SchemaMetadata,
        parent_type_names: &HashSet<String>,
    ) {
        let guarded_parent_types =
            Self::guarded_parent_types(parent_type_names, &parent_field.parent_type_guard);
        let possible_child_types = Self::possible_field_runtime_types(
            parent_field,
            schema_metadata,
            &guarded_parent_types,
        );

        let ProjectionValueSource::ResponseData { selections } = &mut parent_field.value else {
            return;
        };

        let Some(selections_arc) = selections else {
            return;
        };

        let selections_mut = Arc::make_mut(selections_arc);

        for child in selections_mut {
            if child.parent_type_guard.as_ref().is_some_and(|child_guard| {
                Self::type_condition_covers_types(child_guard, &possible_child_types)
            }) {
                child.parent_type_guard = None;
            }

            Self::remove_redundant_child_guards(child, schema_metadata, &possible_child_types);
        }
    }

    fn apply_directive_conditions(
        condition: Option<FieldProjectionCondition>,
        include_if: &Option<String>,
        skip_if: &Option<String>,
    ) -> Option<FieldProjectionCondition> {
        let mut condition = condition;
        if let Some(include_if_var) = include_if {
            condition = Self::and_optional(
                condition,
                Some(FieldProjectionCondition::IncludeIfVariable(
                    include_if_var.clone(),
                )),
            );
        }
        if let Some(skip_if_var) = skip_if {
            condition = Self::and_optional(
                condition,
                Some(FieldProjectionCondition::SkipIfVariable(
                    skip_if_var.clone(),
                )),
            );
        }
        condition
    }

    fn combine_optional<T, F>(left: Option<T>, right: Option<T>, combiner: F) -> Option<T>
    where
        F: FnOnce(T, T) -> T,
    {
        match (left, right) {
            (None, None) => None,
            (Some(c), None) | (None, Some(c)) => Some(c),
            (Some(l), Some(r)) => Some(combiner(l, r)),
        }
    }

    /// Merges optional projection conditions.
    fn or_optional(
        left: Option<FieldProjectionCondition>,
        right: Option<FieldProjectionCondition>,
    ) -> Option<FieldProjectionCondition> {
        match (left, right) {
            // `None` means "always project", so it wins over any condition.
            (None, _) | (_, None) => None,
            (Some(l), Some(r)) => Some(l.or(r)),
        }
    }

    /// Combines two optional conditions with AND logic
    fn and_optional(
        left: Option<FieldProjectionCondition>,
        right: Option<FieldProjectionCondition>,
    ) -> Option<FieldProjectionCondition> {
        Self::combine_optional(left, right, |l, r| l.and(r))
    }

    /// Keeps only the parts of a parent condition that still apply to child selections.
    /// After we move into a field value, checks for the old parent object no longer fit.
    /// Only `@include` and `@skip` checks still make sense.
    fn conditions_for_child_selections(
        condition: &FieldProjectionCondition,
    ) -> Option<FieldProjectionCondition> {
        use FieldProjectionCondition::*;
        match condition {
            IncludeIfVariable(variable_name) => Some(IncludeIfVariable(variable_name.clone())),
            SkipIfVariable(variable_name) => Some(SkipIfVariable(variable_name.clone())),
            And(left, right) => Self::and_optional(
                Self::conditions_for_child_selections(left),
                Self::conditions_for_child_selections(right),
            ),
            Or(left, right) => Self::or_optional(
                Self::conditions_for_child_selections(left),
                Self::conditions_for_child_selections(right),
            ),
            ParentTypeCondition(_) | FieldTypeCondition(_) | EnumValuesCondition(_) => None,
        }
    }

    /// Adds the type guard back when the condition must keep that type scope.
    fn condition_with_optional_guard(
        guard: &Option<TypeCondition>,
        condition: Option<FieldProjectionCondition>,
    ) -> Option<FieldProjectionCondition> {
        match guard {
            None => condition,
            Some(guard) => Some(Self::condition_with_guard(guard, condition)),
        }
    }

    /// Wraps a condition with a ParentType guard, or returns just the guard if no condition.
    /// This creates
    ///   `ParentType(guard) AND condition`,
    /// or just
    ///   `ParentType(guard)`
    /// if condition is None.
    fn condition_with_guard(
        guard: &TypeCondition,
        condition: Option<FieldProjectionCondition>,
    ) -> FieldProjectionCondition {
        let parent_check = FieldProjectionCondition::ParentTypeCondition(guard.clone());
        match condition {
            Some(cond) => parent_check.and(cond),
            None => parent_check,
        }
    }

    /// Merges two field conditions and keeps the parent type scope correct.
    ///
    /// If both conditions use the same scope, they can be merged directly.
    /// If they use different scopes, each condition must keep its own guard.
    fn merge_conditions(
        left_guard: &Option<TypeCondition>,
        left_condition: Option<FieldProjectionCondition>,
        right_guard: &Option<TypeCondition>,
        right_condition: Option<FieldProjectionCondition>,
    ) -> Option<FieldProjectionCondition> {
        // If both guards are the same, both conditions apply in the same type scope.
        // In that case, "always project" wins over a conditional branch.
        if left_guard == right_guard {
            return Self::or_optional(left_condition, right_condition);
        }

        // If the guards are different, each condition must keep its own type scope.
        //
        // Example:
        //   name @include(if: $cond)
        //   ... on User { name }
        //
        // This must become:
        //   Include($cond) OR ParentType(User)
        //
        // We must not drop the conditions and return `None`.
        let left = Self::condition_with_optional_guard(left_guard, left_condition);
        let right = Self::condition_with_optional_guard(right_guard, right_condition);

        Self::or_optional(left, right)
    }

    fn type_conditions_overlap(left: &TypeCondition, right: &TypeCondition) -> bool {
        match (left, right) {
            (TypeCondition::Exact(left), TypeCondition::Exact(right)) => left == right,
            (TypeCondition::Exact(exact), TypeCondition::OneOf(types))
            | (TypeCondition::OneOf(types), TypeCondition::Exact(exact)) => types.contains(exact),
            (TypeCondition::OneOf(left), TypeCondition::OneOf(right)) => {
                left.iter().any(|type_name| right.contains(type_name))
            }
        }
    }

    fn type_condition_from_types(types: HashSet<String>) -> Option<TypeCondition> {
        match types.len() {
            0 => None,
            1 => Some(TypeCondition::Exact(
                types.into_iter().next().expect("set has one element"),
            )),
            _ => Some(TypeCondition::OneOf(types)),
        }
    }

    fn type_condition_types(condition: &TypeCondition) -> HashSet<String> {
        match condition {
            TypeCondition::Exact(type_name) => HashSet::from_iter([type_name.clone()]),
            TypeCondition::OneOf(type_names) => type_names.clone(),
        }
    }

    fn type_condition_intersection(
        left: &TypeCondition,
        right: &TypeCondition,
    ) -> Option<TypeCondition> {
        let mut left_types = Self::type_condition_types(left);
        let right_types = Self::type_condition_types(right);
        left_types.retain(|type_name| right_types.contains(type_name));
        Self::type_condition_from_types(left_types)
    }

    fn type_condition_difference(
        left: &TypeCondition,
        right: &TypeCondition,
    ) -> Option<TypeCondition> {
        let mut left_types = Self::type_condition_types(left);
        let right_types = Self::type_condition_types(right);
        left_types.retain(|type_name| !right_types.contains(type_name));
        Self::type_condition_from_types(left_types)
    }

    fn guards_can_overlap(left: &Option<TypeCondition>, right: &Option<TypeCondition>) -> bool {
        match (left, right) {
            (Some(left), Some(right)) => Self::type_conditions_overlap(left, right),
            // `None` means the field applies to every parent type.
            _ => true,
        }
    }

    fn abstract_parent_type_guard(
        schema_metadata: &SchemaMetadata,
        parent_type_name: &str,
    ) -> Option<TypeCondition> {
        if schema_metadata.is_interface_type(parent_type_name)
            || schema_metadata.is_union_type(parent_type_name)
        {
            Self::type_condition_from_types(Self::possible_runtime_types(
                schema_metadata,
                parent_type_name,
            ))
        } else {
            None
        }
    }

    fn insert_plan(
        field_selections: &mut IndexMap<String, FieldProjectionPlan>,
        plan_to_insert: FieldProjectionPlan,
    ) {
        let response_key = plan_to_insert.response_key.clone();
        if !field_selections.contains_key(&response_key) {
            field_selections.insert(response_key, plan_to_insert);
            return;
        }

        let mut index = field_selections.len();
        loop {
            // IndexMap keys are internal only; projection writes `response_key`.
            // `#` is not legal in GraphQL names or aliases, so this cannot collide
            // with a real response key.
            let internal_key = format!("{response_key}#{index}");
            if !field_selections.contains_key(&internal_key) {
                field_selections.insert(internal_key, plan_to_insert);
                return;
            }
            index += 1;
        }
    }

    fn split_overlapping_guarded_plans(
        field_selections: &mut IndexMap<String, FieldProjectionPlan>,
        existing_index: usize,
        plan_to_merge: FieldProjectionPlan,
    ) {
        let existing_guard = field_selections
            .get_index(existing_index)
            .and_then(|(_, plan)| plan.parent_type_guard.clone())
            .expect("overlapping guarded plan must have an existing guard");
        let incoming_guard = plan_to_merge
            .parent_type_guard
            .clone()
            .expect("overlapping guarded plan must have an incoming guard");

        let overlap = Self::type_condition_intersection(&existing_guard, &incoming_guard)
            .expect("overlapping guarded plans must have an intersection");
        let (_, existing_plan) = field_selections
            .shift_remove_index(existing_index)
            .expect("existing projection plan index must be present");

        if let Some(existing_remainder_guard) =
            Self::type_condition_difference(&existing_guard, &overlap)
        {
            let mut existing_remainder = existing_plan.clone();
            existing_remainder.parent_type_guard = Some(existing_remainder_guard);
            Self::merge_plan(field_selections, existing_remainder);
        }

        if let Some(incoming_remainder_guard) =
            Self::type_condition_difference(&incoming_guard, &overlap)
        {
            let mut incoming_remainder = plan_to_merge.clone();
            incoming_remainder.parent_type_guard = Some(incoming_remainder_guard);
            Self::merge_plan(field_selections, incoming_remainder);
        }

        let mut overlapping_existing = existing_plan;
        overlapping_existing.parent_type_guard = Some(overlap.clone());

        let mut overlapping_incoming = plan_to_merge;
        overlapping_incoming.parent_type_guard = Some(overlap);

        Self::merge_matching_plan(&mut overlapping_existing, overlapping_incoming);
        Self::merge_plan(field_selections, overlapping_existing);
    }

    /// When the same field appears in multiple fragments,
    /// this function combines them into a single plan by:
    /// - Merging equal or unconditional type guards
    /// - Splitting overlapping but unequal type guards before child selections are merged
    /// - OR-ing conditions while preserving guard associations
    /// - Recursively merging child selections
    fn merge_plan(
        field_selections: &mut IndexMap<String, FieldProjectionPlan>,
        plan_to_merge: FieldProjectionPlan,
    ) {
        let existing_index = field_selections.iter().position(|(_, existing_plan)| {
            existing_plan.response_key == plan_to_merge.response_key
                && Self::guards_can_overlap(
                    &existing_plan.parent_type_guard,
                    &plan_to_merge.parent_type_guard,
                )
        });

        let Some(existing_index) = existing_index else {
            // First time seeing this response key, or the same response key belongs to a
            // disjoint type branch. Keep disjoint branches separate so child selections from
            // one concrete type do not leak into another concrete type's field.
            Self::insert_plan(field_selections, plan_to_merge);
            return;
        };

        let should_split =
            field_selections
                .get_index(existing_index)
                .is_some_and(|(_, existing_plan)| {
                    matches!(
                        (&existing_plan.parent_type_guard, &plan_to_merge.parent_type_guard),
                        (Some(existing_guard), Some(incoming_guard))
                            if existing_guard != incoming_guard
                                && Self::type_conditions_overlap(existing_guard, incoming_guard)
                    )
                });

        if should_split {
            Self::split_overlapping_guarded_plans(field_selections, existing_index, plan_to_merge);
            return;
        }

        let (_, existing_plan) = field_selections
            .get_index_mut(existing_index)
            .expect("existing projection plan key must be present");

        Self::merge_matching_plan(existing_plan, plan_to_merge);
    }

    fn merge_matching_plan(
        existing_plan: &mut FieldProjectionPlan,
        plan_to_merge: FieldProjectionPlan,
    ) {
        // Capture guards before merging, needed for condition association
        let existing_guard = existing_plan.parent_type_guard.clone();
        let new_guard = plan_to_merge.parent_type_guard.clone();

        // Merge type guards using OR semantics (union of when to include the field)
        // - None means "applies to all types" (no type restriction)
        // - Some(guard) means "applies only to specific types"
        existing_plan.parent_type_guard = match (
            existing_plan.parent_type_guard.take(),
            plan_to_merge.parent_type_guard,
        ) {
            (None, _) | (_, None) => None, // None (all types) subsumes any specific guard
            (Some(left), Some(right)) => Some(left.union(right)),
        };

        existing_plan.conditions = Self::merge_conditions(
            &existing_guard,
            existing_plan.conditions.take(),
            &new_guard,
            plan_to_merge.conditions,
        );

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
                            let selections_mut = Arc::make_mut(selections);
                            let new_selections_vec = Arc::try_unwrap(new_selections)
                                .unwrap_or_else(|arc| (*arc).clone());

                            // Convert Vec to Map for efficient merging by response_key
                            let mut selections_map: IndexMap<String, FieldProjectionPlan> =
                                selections_mut
                                    .drain(..)
                                    .map(|plan| (plan.response_key.clone(), plan))
                                    .collect();

                            // Recursively merge each child selection
                            for new_plan in new_selections_vec {
                                Self::merge_plan(&mut selections_map, new_plan);
                            }

                            // Convert back to Vec for efficient iteration during projection
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

    fn simplify_condition(
        condition: FieldProjectionCondition,
        parent_type_guard: &Option<TypeCondition>,
    ) -> Option<FieldProjectionCondition> {
        let Some(TypeCondition::Exact(guard_type)) = parent_type_guard else {
            return Some(condition);
        };

        match condition {
            // Remove redundant ParentTypeCondition that matches the guard
            FieldProjectionCondition::ParentTypeCondition(TypeCondition::Exact(cond_type))
                if &cond_type == guard_type =>
            {
                None
            }

            // OneOf with single type matching guard is redundant
            FieldProjectionCondition::ParentTypeCondition(TypeCondition::OneOf(types))
                if types.len() == 1
                    && types.iter().next().map(|t| t.as_str()) == Some(guard_type) =>
            {
                None
            }

            // Recursively simplify AND expressions
            FieldProjectionCondition::And(left, right) => {
                let left_simplified = Self::simplify_condition(*left, parent_type_guard);
                let right_simplified = Self::simplify_condition(*right, parent_type_guard);

                match (left_simplified, right_simplified) {
                    (None, None) => None,
                    (Some(cond), None) | (None, Some(cond)) => Some(cond),
                    (Some(l), Some(r)) => Some(l.and(r)),
                }
            }

            // Recursively simplify OR expressions
            FieldProjectionCondition::Or(left, right) => {
                let left_simplified = Self::simplify_condition(*left, parent_type_guard);
                let right_simplified = Self::simplify_condition(*right, parent_type_guard);

                Self::or_optional(left_simplified, right_simplified)
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
        parent_condition: &Option<FieldProjectionCondition>,
    ) {
        let field_name = &field.name;
        let response_key = field.alias.as_ref().unwrap_or(field_name).clone();

        let (field_type, nullability) = if field_name == TYPENAME_FIELD_NAME {
            ("String".to_string(), FieldNullability::type_name())
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
                Some(f) => (f.output_type_name.clone(), f.nullability.clone()),
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

        let parent_type_guard = parent_condition
            .as_ref()
            .and_then(Self::get_type_guard)
            .or_else(|| Self::abstract_parent_type_guard(schema_metadata, parent_type_name));
        let inherited_selection_conditions = parent_condition
            .as_ref()
            .and_then(Self::conditions_for_child_selections);
        let conditions_for_selections = Self::apply_directive_conditions(
            Self::and_optional(
                inherited_selection_conditions,
                parent_type_guard
                    .as_ref()
                    .map(|_| FieldProjectionCondition::ParentTypeCondition(type_condition.clone())),
            ),
            &field.include_if,
            &field.skip_if,
        );

        let mut condition_for_field = if schema_metadata.is_union_type(&field_type)
            || schema_metadata.is_interface_type(&field_type)
        {
            Self::and_optional(
                parent_condition.clone(),
                Some(FieldProjectionCondition::FieldTypeCondition(type_condition)),
            )
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
            condition_for_field = Self::and_optional(
                condition_for_field,
                Some(FieldProjectionCondition::EnumValuesCondition(
                    enum_values.clone(),
                )),
            );
        }

        let final_conditions =
            condition_for_field.and_then(|cond| Self::simplify_condition(cond, &parent_type_guard));

        let new_plan = if matches!(
            field.selections.items.as_slice(),
            [SelectionItem::Field(FieldSelection {
                omit_from_response: true,
                ..
            })]
        ) {
            // We hit a case where the field is marked as `skip_in_response_projection`,
            // but we still need to project it as an object with no children.
            FieldProjectionPlan {
                field_name: field.name.to_string(),
                response_key,
                parent_type_guard,
                is_typename: field_name == TYPENAME_FIELD_NAME,
                nullability: nullability.clone(),
                conditions: final_conditions,
                // We use Some(vec![]) as it means "project an object, but with no children".
                // None would be treated as "no projection plan available".
                value: ProjectionValueSource::ResponseData {
                    selections: Some(Arc::new(Vec::new())),
                },
            }
        } else {
            FieldProjectionPlan {
                field_name: field_name.to_string(),
                response_key,
                parent_type_guard,
                is_typename: field_name == TYPENAME_FIELD_NAME,
                nullability: nullability.clone(),
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
            }
        };

        Self::merge_plan(field_selections, new_plan);
    }

    fn process_inline_fragment(
        inline_fragment: &InlineFragmentSelection,
        field_selections: &mut IndexMap<String, FieldProjectionPlan>,
        schema_metadata: &SchemaMetadata,
        parent_condition: &Option<FieldProjectionCondition>,
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

        let mut condition_for_fragment = Self::and_optional(
            parent_condition.clone(),
            Some(FieldProjectionCondition::ParentTypeCondition(
                type_condition.clone(),
            )),
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
            nullability: self.nullability.clone(),
            value: new_value,
        }
    }

    fn get_type_guard(condition: &FieldProjectionCondition) -> Option<TypeCondition> {
        match condition {
            FieldProjectionCondition::ParentTypeCondition(tc) => Some(tc.clone()),
            FieldProjectionCondition::And(a, b) => Self::combine_optional(
                Self::get_type_guard(a),
                Self::get_type_guard(b),
                |ga, gb| ga.intersect(gb),
            ),
            FieldProjectionCondition::Or(a, b) => Self::combine_optional(
                Self::get_type_guard(a),
                Self::get_type_guard(b),
                |ga, gb| ga.union(gb),
            ),
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
