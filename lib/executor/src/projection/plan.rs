use ahash::HashSet;
use indexmap::IndexMap;
use std::fmt::{Display, Formatter as FmtFormatter, Result as FmtResult};
use std::sync::Arc;
use tracing::{instrument, trace, warn};

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

/// Selections collected while building a plan, grouped by response key.
///
/// `key` is the field name
/// `value` is the list of possible variants, based on different type guards
///
/// They are kept apart so their nested selections never bleed into one another, and
/// `resolve_variants` later collapses them into mutually-exclusive per-concrete-type plans.
type SelectionVariants = IndexMap<String, Vec<FieldProjectionPlan>>;

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

fn debug_plans_vec(plans: &[FieldProjectionPlan]) {
    for (i, plan) in plans.iter().enumerate() {
        trace!("plan {}:\n{}", i, plan);
    }
}

fn debug_plans_map(plans: &SelectionVariants) {
    for (i, (key, plans)) in plans.iter().enumerate() {
        trace!("key {}: {}", i, key);
        debug_plans_vec(plans);
    }
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
    #[instrument(level = "trace", skip_all)]
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

        for plan in &mut plans {
            Self::remove_redundant_child_guards(plan, schema_metadata);
        }

        (root_type_name, plans)
    }

    #[instrument(level = "trace", skip_all, fields(
      selection_set = %selection_set,
      parent_type_name,
      parent_condition = ?parent_condition,
    ))]
    fn from_selection_set(
        selection_set: &SelectionSet,
        schema_metadata: &SchemaMetadata,
        parent_type_name: &str,
        parent_condition: &Option<FieldProjectionCondition>,
    ) -> Option<Vec<FieldProjectionPlan>> {
        let mut field_selections = SelectionVariants::new();

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
                        parent_type_name,
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
            return None;
        }

        let resolved = Self::resolve_variants(field_selections, parent_type_name, schema_metadata);

        if resolved.is_empty() {
            None
        } else {
            Some(resolved)
        }
    }

    /// Collapses each response key's variants into the final selections.
    ///
    /// A response key may be selected for multiple parent type with
    /// different nested selections:
    ///
    /// `... on Article { meta { title } }`
    /// `... on Video { meta { wordCount } }`
    ///
    /// `merge_plan` keeps those as separate selections so their nested selections
    /// never "bleed" into one another.
    ///
    /// The end goal here is to merged the variants, so a field `meta` will not
    /// have 2 plan records, but rather it will be one record, with multiple type guards.
    ///
    /// This way, at runtime, the `meta` plan will match exactly one variant, based on the concrete `__typename`
    #[instrument(level = "trace", skip_all, fields(parent_type_name,))]
    fn resolve_variants(
        field_selections: SelectionVariants,
        parent_type_name: &str,
        schema_metadata: &SchemaMetadata,
    ) -> Vec<FieldProjectionPlan> {
        #[cfg(debug_assertions)]
        {
            trace!("input:\n");
            debug_plans_map(&field_selections);
        }
        let mut concrete_types: Option<Vec<String>> = None;
        let mut resolved = Vec::new();

        for (_, variants) in field_selections {
            let already_exclusive = variants.len() == 1
                || variants
                    .iter()
                    .all(|v| matches!(v.parent_type_guard, Some(TypeCondition::Exact(_))));
            if already_exclusive {
                resolved.extend(variants);
                continue;
            }

            let concrete_types = concrete_types.get_or_insert_with(|| {
                let mut types: Vec<String> = schema_metadata
                    .possible_types
                    .get_possible_types(parent_type_name)
                    .into_iter()
                    .filter(|t| schema_metadata.is_object_type(t))
                    .collect();
                types.sort();
                types
            });

            for concrete_type in concrete_types.iter() {
                let merged = Self::merge_variants_for_type(
                    &variants,
                    concrete_type,
                    parent_type_name,
                    schema_metadata,
                );
                resolved.extend(merged);
            }
        }

        #[cfg(debug_assertions)]
        {
            trace!("output:\n");
            debug_plans_vec(&resolved);
        }

        resolved
    }

    /// Merges every variant whose guard matches `concrete_type` into a single
    /// plan scoped to `Exact(concrete_type)`. Returns `None` when no variant
    /// applies to that type.
    fn merge_variants_for_type(
        variants: &[FieldProjectionPlan],
        concrete_type: &str,
        parent_type_name: &str,
        schema_metadata: &SchemaMetadata,
    ) -> Option<FieldProjectionPlan> {
        let mut merged: Option<FieldProjectionPlan> = None;
        for variant in variants {
            let matches = variant
                .parent_type_guard
                .as_ref()
                .is_none_or(|guard| guard.matches(concrete_type));
            if !matches {
                continue;
            }
            let mut scoped = variant.clone();
            scoped.parent_type_guard = Some(TypeCondition::Exact(concrete_type.to_string()));
            match &mut merged {
                None => merged = Some(scoped),
                Some(existing) => {
                    Self::merge_matching(existing, scoped, parent_type_name, schema_metadata)
                }
            }
        }
        merged
    }

    fn field_output_type<'a>(
        schema_metadata: &'a SchemaMetadata,
        parent_type_name: &'a str,
        guard: &'a Option<TypeCondition>,
        field_name: &str,
    ) -> Option<&'a str> {
        if field_name == TYPENAME_FIELD_NAME {
            return None;
        }
        let lookup = |candidate: &str| {
            schema_metadata
                .type_fields
                .get(candidate)
                .and_then(|fields| fields.get(field_name))
                .map(|info| info.output_type_name.as_str())
        };
        match guard {
            Some(guard) => Self::type_names_from(guard).into_iter().find_map(lookup),
            None => lookup(parent_type_name),
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
    /// A type guard is redundant when it covers exactly all possible types that can appear in that position.
    /// For example, if a field `children` can only return types A or B (based on the parent's type guard),
    /// and a child selection has guard `OneOf(A, B)`, then that guard is redundant since it will always match.
    ///
    /// Every guard has a cost during execution (computes field's output type or name of its parent),
    /// so removing redundant guards helps optimize the response projection.
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
            HashSet::from_iter(parent_types.iter().filter_map(|parent_type| {
                schema_metadata
                    .type_fields
                    .get(*parent_type)
                    .and_then(|fields| fields.get(&parent_field.field_name))
                    .map(|field_info| field_info.output_type_name.as_str())
            }))
        });

        for child in selections_mut {
            // Remove redundant guard if this child's guard covers all possible types
            if let (Some(child_guard), Some(possible_types)) =
                (&child.parent_type_guard, &possible_child_types)
            {
                // Check if guard covers exactly all possible types by comparing as sets
                let is_redundant = match child_guard {
                    TypeCondition::Exact(ty) => {
                        possible_types.len() == 1 && possible_types.contains(ty.as_str())
                    }
                    TypeCondition::OneOf(guard_types) => {
                        guard_types.len() == possible_types.len()
                            && guard_types
                                .iter()
                                .all(|t| possible_types.contains(t.as_str()))
                    }
                };

                if is_redundant {
                    child.parent_type_guard = None;
                }
            }

            Self::remove_redundant_child_guards(child, schema_metadata);
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

    /// Adds a plan to the selection map. Selections are grouped by response key,
    /// and within a key kept as separate variants per parent type guard.
    ///
    /// `merge_plan` will add a common-field (a field that's relevant to multiple plans) to
    /// both plans.
    ///
    /// A later process, `resolve_variants` processes the common fields and "bubble" it if possible.
    #[instrument(level = "trace", skip_all, fields(parent_type_name))]
    fn merge_plan(
        field_selections: &mut SelectionVariants,
        plan_to_merge: FieldProjectionPlan,
        parent_type_name: &str,
        schema_metadata: &SchemaMetadata,
    ) {
        let Some(variants) = field_selections.get_mut(plan_to_merge.response_key.as_str()) else {
            field_selections.insert(plan_to_merge.response_key.clone(), vec![plan_to_merge]);
            return;
        };

        match variants
            .iter_mut()
            .find(|v| v.parent_type_guard == plan_to_merge.parent_type_guard)
        {
            Some(existing) => {
                Self::merge_matching(existing, plan_to_merge, parent_type_name, schema_metadata)
            }
            None => variants.push(plan_to_merge),
        }
    }

    /// Merges a plan into an existing one with the same response key and the same type guard
    #[instrument(level = "trace", skip_all, fields(parent_type_name))]
    fn merge_matching(
        existing_plan: &mut FieldProjectionPlan,
        plan_to_merge: FieldProjectionPlan,
        parent_type_name: &str,
        schema_metadata: &SchemaMetadata,
    ) {
        // Same guard on both sides, so there is nothing to union here.
        existing_plan.conditions = Self::merge_conditions(
            &existing_plan.parent_type_guard,
            existing_plan.conditions.take(),
            &existing_plan.parent_type_guard,
            plan_to_merge.conditions,
        );

        // Children live one level down, under this field's output type.
        let child_parent_type = Self::field_output_type(
            schema_metadata,
            parent_type_name,
            &existing_plan.parent_type_guard,
            &existing_plan.field_name,
        )
        .unwrap_or(parent_type_name);

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

                            // Merge children, keeping variants per guard separate.
                            let mut child_selections = SelectionVariants::new();
                            for child in selections_mut.drain(..) {
                                child_selections
                                    .entry(child.response_key.clone())
                                    .or_default()
                                    .push(child);
                            }
                            for new_plan in new_selections_vec {
                                Self::merge_plan(
                                    &mut child_selections,
                                    new_plan,
                                    child_parent_type,
                                    schema_metadata,
                                );
                            }

                            // Re-resolve in case merging introduced overlapping variants.
                            selections_mut.extend(Self::resolve_variants(
                                child_selections,
                                child_parent_type,
                                schema_metadata,
                            ));
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

    #[instrument(level = "trace", skip_all, fields(
      field_name = field.name,
      field_alias = field.alias.as_deref(),
      parent_type_name,
      parent_condition = ?parent_condition,
    ))]
    fn process_field(
        field: &FieldSelection,
        field_selections: &mut SelectionVariants,
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

        let parent_type_guard = parent_condition.as_ref().and_then(Self::get_type_guard);
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

        Self::merge_plan(
            field_selections,
            new_plan,
            parent_type_name,
            schema_metadata,
        );
    }

    #[instrument(level = "trace", skip_all, fields(
      inline_fragment_type = inline_fragment.type_condition,
      fragment_str = %inline_fragment.selections,
      parent_type_name,
      parent_condition = ?parent_condition,
    ))]
    fn process_inline_fragment(
        inline_fragment: &InlineFragmentSelection,
        field_selections: &mut SelectionVariants,
        schema_metadata: &SchemaMetadata,
        parent_type_name: &str,
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
                Self::merge_plan(
                    field_selections,
                    selection,
                    parent_type_name,
                    schema_metadata,
                );
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
                let mut types_vec: Vec<_> = types.iter().collect();
                types_vec.sort(); // for consistent output

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

#[cfg(test)]
mod tests {
    use super::FieldProjectionPlan;
    use crate::introspection::schema::SchemaWithMetadata;
    use hive_router_query_planner::{
        ast::normalization::normalize_operation,
        consumer_schema::ConsumerSchema,
        state::supergraph_state::SupergraphState,
        utils::parsing::{parse_operation, parse_schema},
    };

    const SCHEMA_1: &str = r#"
        type Query {
            search: [SearchResult!]!
            feed: [Content!]!
            concrete: ConcreteType
        }

        type ConcreteType {
            id: ID!
            test: String!
            someEnum: SomeEnum
            inner: InnerType
        }

        enum SomeEnum {
            TEST
        }

        type InnerType {
            inner: Boolean
        }

        union SearchResult = Article | Video

        interface Content {
            id: ID!
            meta: Meta!
        }

        type Article implements Content {
            id: ID!
            meta: Meta!
            headline: String!
        }

        type Video implements Content {
            id: ID!
            meta: Meta!
            duration: Int!
        }

        type Photo implements Content {
            id: ID!
            meta: Meta!
        }

        type Meta {
            title: String
            wordCount: Int!
            arch: String!
        }
    "#;

    fn plan(schema: &str, query: &str) -> String {
        let supergraph = parse_schema(schema);
        let consumer_schema = ConsumerSchema::new_from_supergraph(&supergraph);
        let schema_metadata = consumer_schema.schema_metadata();

        let operation = parse_operation(query);
        let supergraph_state = SupergraphState::new(&supergraph);
        let normalized =
            normalize_operation(&supergraph_state, &operation, None).expect("failed to normalize");
        println!("{}", normalized.operation);
        let (_, selections) =
            FieldProjectionPlan::from_operation(&normalized.operation, &schema_metadata);

        selections
            .iter()
            .map(|p| p.to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn simple_concrete_mixed_fields_defs() {
        insta::assert_snapshot!(plan(SCHEMA_1,
            r#"{
                concrete {
                    id
                    test
                    someEnum
                    inner {
                        inner
                    }
                }
            }"#
        ), @r###"
        concrete: {
          selections:
            id: {
            }
            test: {
            }
            someEnum: {
              conditions: EnumValues(TEST)
            }
            inner: {
              selections:
                inner: {
                }
            }
        }
        "###);
    }

    #[test]
    fn inline_fragment_on_concrete_type_with_dups() {
        insta::assert_snapshot!(plan(SCHEMA_1,
            r#"{
                concrete {
                  ... on ConcreteType {
                    id
                    inner {
                        inner
                    }
                  }
                  test
                  someEnum
                  inner {
                      inner
                  }
                }
            }"#
        ), @r###"
        concrete: {
          selections:
            id: {
            }
            inner: {
              selections:
                inner: {
                }
            }
            test: {
            }
            someEnum: {
              conditions: EnumValues(TEST)
            }
        }
        "###);
    }

    /// #1166: union members select the shared key `meta` with disjoint
    /// sub-fields.
    #[test]
    fn union_disjoint_fragments_kept_per_type() {
        insta::assert_snapshot!(plan(SCHEMA_1,
            r#"{
                search {
                    __typename
                    ... on Article { meta { title wordCount } }
                    ... on Video { meta { title arch } }
                }
            }"#
        ), @r###"
        search: {
          conditions: FieldType(OneOf(Article, SearchResult, Video))
          selections:
            __typename: {
            }
            meta: {
              type guard: Exact(Article)
              selections:
                title: {
                }
                wordCount: {
                }
            }
            meta: {
              type guard: Exact(Video)
              selections:
                title: {
                }
                arch: {
                }
            }
        }
        "###);
    }

    /// An "unguarded" selection (no concrete type) of `meta` (on the `Content` interface) overlaps a
    /// guarded one (`... on Article`).
    ///
    /// The overlap is split per concrete type:
    /// `Article` -> merges both
    /// `Video`/`Photo` keep only the shared field
    #[test]
    fn interface_overlapping_guard_splits_per_type() {
        insta::assert_snapshot!(plan(SCHEMA_1,
            r#"{
                feed {
                    meta { title }
                    ... on Article { meta { wordCount } }
                }
            }"#
        ), @r###"
        feed: {
          conditions: FieldType(OneOf(Article, Content, Photo, Video))
          selections:
            meta: {
              type guard: Exact(Article)
              selections:
                title: {
                }
                wordCount: {
                }
            }
            meta: {
              type guard: Exact(Photo)
              selections:
                title: {
                }
            }
            meta: {
              type guard: Exact(Video)
              selections:
                title: {
                }
            }
        }
        "###);
    }

    /// Same key, same guard, same selection from two fragments collapses back
    /// into a single plan (no duplications).
    ///
    /// DOTAN: Added this because I was concered that doing split/merge of the plans based of type,
    /// might end up with duplicate projections, now that it's no longer a single one.
    #[test]
    fn same_guard_fragments_merge_into_one() {
        insta::assert_snapshot!(plan(SCHEMA_1,
            r#"{
                search {
                    ... on Article { meta { title } }
                    ... on Article { meta { title } }
                }
            }"#
        ), @r###"
        search: {
          conditions: FieldType(OneOf(Article, SearchResult, Video))
          selections:
            meta: {
              type guard: Exact(Article)
              selections:
                title: {
                }
            }
        }
        "###);
    }

    const SCHEMA_2: &str = r#"
    enum WeightUnit { KG LB G }

    interface Node {
      id: ID!
    }

    interface Animal implements Node {
      id: ID!
      name: String!
    }

    interface Pet implements Animal & Node {
      id: ID!
      name: String!
      bestFriend: Animal
      weight(unit: WeightUnit = KG): Float
    }

    type Dog implements Pet & Animal & Node {
      id: ID!
      name: String!
      bestFriend: Animal
      weight(unit: WeightUnit = KG): Float
      nickname: String
      tags: [String!]
    }

    type Cat implements Pet & Animal & Node {
      id: ID!
      name: String!
      bestFriend: Cat                     # covariant override
      weight(unit: WeightUnit = KG): Float
      age: Int
      tags: [String!]
    }

    type Robot implements Node {
      id: ID!
      model: String!
      weight(unit: WeightUnit = KG): Float
    }

    union SearchResult = Dog | Cat | Robot

    type Owner implements Node {
      id: ID!
      name: String!
      pets: [Pet!]!
      primaryPet: Pet
    }

    type Query {
      pet: Pet
      animal: Animal
      node: Node
      search: SearchResult
      searchMany: [SearchResult!]!
      pets: [Pet!]!
      owner: Owner
    }
    "#;

    /// When the fragment's id merges into the existing id entry, the key keeps the position of its
    /// first occurrence — it does not move to after nickname.
    #[test]
    fn field_order_when_grouping_with_fragments() {
        let plan = plan(
            SCHEMA_2,
            r#"
        query FieldOrder {
          animal {
            name
            id
            ... on Dog {
              id
              nickname
            }
          }
        }"#,
        );

        insta::assert_snapshot!(plan, @r###"
        animal: {
          conditions: FieldType(OneOf(Animal, Cat, Dog, Pet))
          selections:
            name: {
            }
            id: {
              type guard: Exact(Cat)
            }
            id: {
              type guard: Exact(Dog)
            }
            nickname: {
              type guard: Exact(Dog)
            }
        }
        "###);
    }

    #[test]
    fn multiple_aliases() {
        let plan = plan(
            SCHEMA_2,
            r#"
            query AliasFanout {
              pet {
                ... on Dog {
                  kg: weight(unit: KG)
                  lb: weight(unit: LB)
                  g:  weight(unit: G)
                }
                ... on Cat {
                  kg: weight(unit: KG)
                }
              }
            }"#,
        );

        insta::assert_snapshot!(plan, @r###"
        pet: {
          conditions: FieldType(OneOf(Cat, Dog, Pet))
          selections:
            kg (alias for weight) {
              type guard: Exact(Dog)
            }
            kg (alias for weight) {
              type guard: Exact(Cat)
            }
            lb (alias for weight) {
              type guard: Exact(Dog)
            }
            g (alias for weight) {
              type guard: Exact(Dog)
            }
        }
        "###);
    }

    // #[test]
    // fn nested_redundant_fragments() {
    //     let plan = plan(
    //         SCHEMA_2,
    //         r#"
    //         query M_IdentityConditions {
    //           pet {
    //             ... on Pet {
    //               ... on Pet {
    //                 name
    //                 ... on Animal { id }
    //               }
    //             }
    //           }
    //         }"#,
    //     );

    //     insta::assert_snapshot!(plan, @r###"
    //     pet: {
    //       conditions: FieldType(OneOf(Cat, Dog, Pet))
    //       selections:
    //         name: {
    //         }
    //     }
    //     "###);
    // }

    // Directive on a named fragment spread, with a field merged across two fragments
    // #[test]
    // fn directive_on_named_fragment_spread_with_field_merged_across_two_fragments() {
    //     let plan = plan(
    //         SCHEMA_2,
    //         r#"
    //         query SpreadDirective($withName: Boolean!, $deep: Boolean!) {
    //           node {
    //             id
    //             ...AnimalBits @include(if: $withName)
    //             ... on Dog @skip(if: $deep) { nickname }
    //           }
    //         }

    //         fragment AnimalBits on Animal {
    //           name
    //           ... on Dog { nickname }
    //         }"#,
    //     );

    //     insta::assert_snapshot!(plan, @r###""###);
    // }
}
