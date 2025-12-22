use std::collections::HashSet;

use hive_router_plan_executor::projection::plan::{FieldProjectionPlan, ProjectionValueSource};
use hive_router_query_planner::ast::{
    operation::OperationDefinition, selection_item::SelectionItem, selection_set::SelectionSet,
    value::Value,
};
use indexmap::IndexMap;

use crate::pipeline::authorization::tree::{PathIndex, UnauthorizedPathTrie};

/// Reconstructs a GraphQL operation with unauthorized fields removed.
pub(super) fn rebuild_authorized_operation<'op>(
    original_operation: &'op OperationDefinition,
    unauthorized_path_trie: &UnauthorizedPathTrie<'op>,
) -> OperationDefinition {
    let selection_set = rebuild_authorized_selection_set(
        &original_operation.selection_set,
        unauthorized_path_trie,
        PathIndex::root(),
    );

    // Collect variables from the filtered operation
    let used_variables = collect_used_variables(&selection_set);

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

/// Recursively filters a selection set to remove unauthorized fields.
fn rebuild_authorized_selection_set<'op>(
    original_selection_set: &'op SelectionSet,
    unauthorized_path_trie: &UnauthorizedPathTrie<'op>,
    path_position: PathIndex,
) -> SelectionSet {
    if !unauthorized_path_trie.has_unauthorized_fields(path_position) {
        return original_selection_set.clone();
    }

    let mut authorized_items = Vec::with_capacity(original_selection_set.items.len());

    for selection in &original_selection_set.items {
        match selection {
            SelectionItem::Field(field) => {
                let path_segment = field.alias.as_ref().unwrap_or(&field.name);

                let Some((child_path_position, is_unauthorized)) =
                    unauthorized_path_trie.find_field(path_position, path_segment)
                else {
                    authorized_items.push(selection.clone());
                    continue;
                };

                if is_unauthorized {
                    continue;
                }

                let filtered_selections = rebuild_authorized_selection_set(
                    &field.selections,
                    unauthorized_path_trie,
                    child_path_position,
                );
                if filtered_selections.is_empty() && !field.selections.is_empty() {
                    continue;
                }

                authorized_items.push(SelectionItem::Field(
                    field.with_new_selections(filtered_selections),
                ));
            }
            SelectionItem::InlineFragment(fragment) => {
                let Some((fragment_path_position, is_unauthorized)) =
                    unauthorized_path_trie.find_field(path_position, &fragment.type_condition)
                else {
                    // If the fragment is not in the trie, it means it's authorized.
                    authorized_items.push(selection.clone());
                    continue;
                };

                // If the fragment's type condition itself is marked as unauthorized, skip it entirely.
                if is_unauthorized {
                    continue;
                }

                let filtered_selections = rebuild_authorized_selection_set(
                    &fragment.selections,
                    unauthorized_path_trie,
                    fragment_path_position,
                );

                if !filtered_selections.is_empty() {
                    authorized_items.push(SelectionItem::InlineFragment(
                        fragment.with_new_selections(filtered_selections),
                    ));
                }
            }
            SelectionItem::FragmentSpread(_) => {
                // Fragment spreads are inlined during normalization, so they shouldn't exist here
            }
        }
    }

    SelectionSet {
        items: authorized_items,
    }
}

/// Rebuilds the projection plan to exclude unauthorized fields.
pub(super) fn rebuild_authorized_projection_plan(
    original_plans: &IndexMap<String, FieldProjectionPlan>,
    unauthorized_path_trie: &UnauthorizedPathTrie,
) -> IndexMap<String, FieldProjectionPlan> {
    rebuild_authorized_projection_plan_recursive(
        original_plans,
        unauthorized_path_trie,
        PathIndex::root(),
    )
    .unwrap_or_default()
}

/// Recursively filters projection plans. Unauthorized fields become null.
fn rebuild_authorized_projection_plan_recursive(
    original_plans: &IndexMap<String, FieldProjectionPlan>,
    unauthorized_path_trie: &UnauthorizedPathTrie,
    path_position: PathIndex,
) -> Option<IndexMap<String, FieldProjectionPlan>> {
    let mut authorized_plans = IndexMap::with_capacity(original_plans.len());

    for (path_segment, plan) in original_plans {
        let Some((child_path_position, is_unauthorized)) =
            unauthorized_path_trie.find_field(path_position, path_segment)
        else {
            authorized_plans.insert(path_segment.clone(), plan.clone());
            continue;
        };

        if is_unauthorized {
            authorized_plans.insert(
                path_segment.clone(),
                plan.with_new_value(ProjectionValueSource::Null),
            );
            continue;
        }

        let new_value = match &plan.value {
            ProjectionValueSource::ResponseData {
                selections: Some(selections),
            } => ProjectionValueSource::ResponseData {
                selections: rebuild_authorized_projection_plan_recursive(
                    selections,
                    unauthorized_path_trie,
                    child_path_position,
                ),
            },
            other => other.clone(),
        };
        authorized_plans.insert(path_segment.clone(), plan.with_new_value(new_value));
    }

    if authorized_plans.is_empty() {
        None
    } else {
        Some(authorized_plans)
    }
}

/// Collects all variable references from a selection set (single-pass).
///
/// Scans the filtered operation to find which variables are actually used,
/// allowing us to remove unused variable definitions.
fn collect_used_variables(selection_set: &SelectionSet) -> HashSet<String> {
    let mut used_variables = HashSet::default();
    collect_variables_recursive(selection_set, &mut used_variables);
    used_variables
}

/// Recursively collects variables from a selection set.
fn collect_variables_recursive(selection_set: &SelectionSet, used_variables: &mut HashSet<String>) {
    for item in &selection_set.items {
        match item {
            SelectionItem::Field(field) => {
                // Collect from field arguments
                if let Some(args) = &field.arguments {
                    for arg in args.values() {
                        collect_variables_from_value(arg, used_variables);
                    }
                }

                // Collect from @skip directive
                if let Some(var_name) = &field.skip_if {
                    used_variables.insert(var_name.clone());
                }

                // Collect from @include directive
                if let Some(var_name) = &field.include_if {
                    used_variables.insert(var_name.clone());
                }

                // Recurse into field selections
                collect_variables_recursive(&field.selections, used_variables);
            }
            SelectionItem::InlineFragment(fragment) => {
                // Collect from fragment directives
                if let Some(var_name) = &fragment.skip_if {
                    used_variables.insert(var_name.clone());
                }
                if let Some(var_name) = &fragment.include_if {
                    used_variables.insert(var_name.clone());
                }

                // Recurse into fragment selections
                collect_variables_recursive(&fragment.selections, used_variables);
            }
            SelectionItem::FragmentSpread(_) => {
                // Fragment spreads are inlined during normalization
            }
        }
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
