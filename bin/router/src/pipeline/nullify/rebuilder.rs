use std::sync::Arc;

use ahash::HashSet;
use hive_router_plan_executor::projection::plan::{FieldProjectionPlan, ProjectionValueSource};
use hive_router_query_planner::ast::{
    operation::OperationDefinition,
    selection_item::SelectionItem,
    selection_set::{FieldSelection, InlineFragmentSelection, SelectionSet},
    value::Value,
};

use crate::pipeline::trie::{PathIndex, Trie};

/// Reconstructs a GraphQL operation with nulled fields removed.
pub(crate) fn rebuild_nulled_operation(
    original_operation: &OperationDefinition,
    nulled_field_trie: &Trie,
) -> OperationDefinition {
    let mut used_variables = HashSet::default();
    let selection_set = rebuild_nulled_selection_set(
        &original_operation.selection_set,
        nulled_field_trie,
        PathIndex::root(),
        &mut used_variables,
    );

    let variable_definitions = original_operation
        .variable_definitions
        .as_ref()
        .map(|var_defs| {
            var_defs
                .iter()
                .filter(|var_def| used_variables.contains(&var_def.name))
                .cloned()
                .collect()
        });

    OperationDefinition {
        name: original_operation.name.clone(),
        operation_kind: original_operation.operation_kind.clone(),
        selection_set,
        variable_definitions,
    }
}

/// Recursively filters a selection set to remove nulled fields, collecting
/// the set of variables used by whatever is kept along the way (so a second
/// full traversal isn't needed afterwards to compute this).
fn rebuild_nulled_selection_set(
    original_selection_set: &SelectionSet,
    nulled_field_trie: &Trie,
    path_position: PathIndex,
    used_variables: &mut HashSet<String>,
) -> SelectionSet {
    // If the current position, is the last one, there are no children to traverse.
    if !nulled_field_trie.has_children(path_position) {
        collect_variables_recursive(original_selection_set, used_variables);
        return original_selection_set.clone();
    }

    let mut kept_items = Vec::with_capacity(original_selection_set.items.len());

    for selection in &original_selection_set.items {
        match selection {
            SelectionItem::Field(field) => {
                let path_segment = field.alias.as_ref().unwrap_or(&field.name);

                let Some((child_path_position, is_nulled)) =
                    nulled_field_trie.find_segment_at_position(path_position, path_segment)
                else {
                    collect_field_own_variables(field, used_variables);
                    collect_variables_recursive(&field.selections, used_variables);
                    kept_items.push(selection.clone());
                    continue;
                };

                if is_nulled {
                    continue;
                }

                let filtered_selections = rebuild_nulled_selection_set(
                    &field.selections,
                    nulled_field_trie,
                    child_path_position,
                    used_variables,
                );
                if filtered_selections.is_empty() && !field.selections.is_empty() {
                    continue;
                }

                collect_field_own_variables(field, used_variables);
                kept_items.push(SelectionItem::Field(
                    field.with_new_selections(filtered_selections),
                ));
            }
            SelectionItem::InlineFragment(fragment) => {
                let filtered_selections = rebuild_nulled_selection_set(
                    &fragment.selections,
                    nulled_field_trie,
                    path_position,
                    used_variables,
                );

                if filtered_selections.is_empty() && !fragment.selections.is_empty() {
                    continue;
                }

                collect_fragment_own_variables(fragment, used_variables);
                kept_items.push(SelectionItem::InlineFragment(
                    fragment.with_new_selections(filtered_selections),
                ));
            }
            SelectionItem::FragmentSpread(_) => {
                // Fragment spreads are inlined during normalization, so they shouldn't exist here
            }
        }
    }

    SelectionSet { items: kept_items }
}

/// Rebuilds the projection plan to set nulled fields to null.
pub(crate) fn rebuild_nulled_projection_plan(
    original_plans: &Vec<FieldProjectionPlan>,
    nulled_field_trie: &Trie,
) -> Vec<FieldProjectionPlan> {
    rebuild_nulled_projection_plan_recursive(original_plans, nulled_field_trie, PathIndex::root())
        .unwrap_or_default()
}

/// Recursively filters projection plans. Nulled fields become null.
fn rebuild_nulled_projection_plan_recursive(
    original_plans: &Vec<FieldProjectionPlan>,
    nulled_field_trie: &Trie,
    path_position: PathIndex,
) -> Option<Vec<FieldProjectionPlan>> {
    let mut kept_plans = Vec::with_capacity(original_plans.len());

    for plan in original_plans {
        let path_segment = &plan.response_key;
        let Some((child_path_position, is_nulled)) =
            nulled_field_trie.find_segment_at_position(path_position, path_segment)
        else {
            kept_plans.push(plan.clone());
            continue;
        };

        if is_nulled {
            kept_plans.push(plan.with_new_value(ProjectionValueSource::Null));
            continue;
        }

        let new_value = match &plan.value {
            ProjectionValueSource::ResponseData {
                selections: Some(selections),
            } => ProjectionValueSource::ResponseData {
                selections: rebuild_nulled_projection_plan_recursive(
                    selections,
                    nulled_field_trie,
                    child_path_position,
                )
                .map(Arc::new),
            },
            other => other.clone(),
        };
        kept_plans.push(plan.with_new_value(new_value));
    }

    if kept_plans.is_empty() {
        None
    } else {
        Some(kept_plans)
    }
}

/// Recursively collects variables from an entire (unfiltered) selection set.
/// Used when a subtree is kept as-is, so it still needs to be scanned once
/// for variable usage.
fn collect_variables_recursive(selection_set: &SelectionSet, used_variables: &mut HashSet<String>) {
    for item in &selection_set.items {
        match item {
            SelectionItem::Field(field) => {
                collect_field_own_variables(field, used_variables);
                collect_variables_recursive(&field.selections, used_variables);
            }
            SelectionItem::InlineFragment(fragment) => {
                collect_fragment_own_variables(fragment, used_variables);
                collect_variables_recursive(&fragment.selections, used_variables);
            }
            SelectionItem::FragmentSpread(_) => {
                // Fragment spreads are inlined during normalization
            }
        }
    }
}

/// Collects variables referenced directly on a field (arguments, `@skip`/`@include`),
/// without recursing into its nested selections.
fn collect_field_own_variables(field: &FieldSelection, used_variables: &mut HashSet<String>) {
    if let Some(args) = &field.arguments {
        for arg in args.values() {
            collect_variables_from_value(arg, used_variables);
        }
    }

    if let Some(var_name) = &field.skip_if {
        used_variables.insert(var_name.clone());
    }

    if let Some(var_name) = &field.include_if {
        used_variables.insert(var_name.clone());
    }
}

/// Collects variables referenced directly on an inline fragment (`@skip`/`@include`),
/// without recursing into its nested selections.
fn collect_fragment_own_variables(
    fragment: &InlineFragmentSelection,
    used_variables: &mut HashSet<String>,
) {
    if let Some(var_name) = &fragment.skip_if {
        used_variables.insert(var_name.clone());
    }
    if let Some(var_name) = &fragment.include_if {
        used_variables.insert(var_name.clone());
    }
}

/// Recursively extracts variable references from GraphQL values.
fn collect_variables_from_value(value: &Value, used_variables: &mut HashSet<String>) {
    match value {
        Value::Variable(var_name) => {
            used_variables.insert(var_name.clone());
        }
        Value::List(items) => {
            for item in items {
                collect_variables_from_value(item, used_variables);
            }
        }
        Value::Object(fields) => {
            for val in fields.values() {
                collect_variables_from_value(val, used_variables);
            }
        }
        Value::Null
        | Value::Int(_)
        | Value::Float(_)
        | Value::String(_)
        | Value::Boolean(_)
        | Value::Enum(_) => {
            // Primitive values don't contain variables
        }
    }
}
