use std::collections::{HashMap, HashSet};
use std::fmt::Write;

use indexmap::IndexMap;
use query_planner::{
    ast::{
        operation::OperationDefinition, selection_item::SelectionItem, selection_set::SelectionSet,
    },
    state::supergraph_state::OperationKind,
};
use serde_json::{Map, Value};
use tracing::{instrument, warn};

use crate::{
    json_writer::write_and_escape_string, schema_metadata::SchemaMetadata, GraphQLError,
    TYPENAME_FIELD,
};

#[derive(Debug, Clone)]
pub struct ProjectionFieldSelection {
    field_name: String,
    response_key: String,
    type_name: String,
    include_if: Option<String>,
    skip_if: Option<String>,
    parent_type_conditions: Option<HashSet<String>>,
    enum_values: Option<HashSet<String>>,
    selections: Option<Vec<ProjectionFieldSelection>>,
}

impl ProjectionFieldSelection {
    pub fn from_selection_set(
        selection_set: &SelectionSet,
        parent_type_name: &str,
        parent_type_conditions: Option<HashSet<String>>,
        schema_metadata: &SchemaMetadata,
        include_if: Option<String>,
        skip_if: Option<String>,
    ) -> Vec<ProjectionFieldSelection> {
        let mut field_selections: IndexMap<String, ProjectionFieldSelection> = IndexMap::new();
        for selection_item in &selection_set.items {
            match selection_item {
                SelectionItem::Field(field) => {
                    let response_key = field.alias.as_ref().unwrap_or(&field.name);
                    let field_name = field.name.clone();
                    let field_type = schema_metadata
                        .type_fields
                        .get(parent_type_name)
                        .and_then(|fields| fields.get(&field_name))
                        .map(|s| s.as_str())
                        .unwrap_or("Any");
                    let mut final_include_if = None;
                    let mut final_skip_if = None;

                    if let Some(include_if) = &include_if {
                        final_include_if = Some(include_if.clone());
                    }
                    if let Some(field_include_if) = &field.include_if {
                        final_include_if = Some(field_include_if.clone());
                    }

                    if let Some(skip_if) = skip_if.clone() {
                        final_skip_if = Some(skip_if);
                    }
                    if let Some(field_skip_if) = &field.skip_if {
                        final_skip_if = Some(field_skip_if.clone());
                    }

                    if let (Some(skip_if), Some(include_if)) = (&final_skip_if, &final_include_if) {
                        if skip_if == include_if {
                            continue;
                        }
                    }

                    if let Some(existing_field) = field_selections.get_mut(response_key.as_str()) {
                        if let Some(existing_conditions) =
                            &mut existing_field.parent_type_conditions
                        {
                            if let Some(ref parent_type_conditions) = parent_type_conditions {
                                existing_conditions.extend(parent_type_conditions.clone());
                            }
                        }
                        if let (Some(existing_field_include_if), Some(field_include_if)) =
                            (&existing_field.include_if, &field.include_if)
                        {
                            if existing_field_include_if != field_include_if {
                                existing_field.include_if = None;
                            }
                        }
                        if let (Some(_existing_field_skip_if), Some(_new_skip_if)) =
                            (&existing_field.skip_if, &field.skip_if)
                        {
                            if existing_field.skip_if != field.skip_if {
                                existing_field.skip_if = None;
                            }
                        }
                        if field.selections.items.is_empty() {
                            existing_field.selections = None;
                        } else {
                            existing_field
                                .selections
                                .get_or_insert_with(Vec::new)
                                .extend(ProjectionFieldSelection::from_selection_set(
                                    &field.selections,
                                    field_type,
                                    parent_type_conditions.clone(),
                                    schema_metadata,
                                    final_include_if.clone(),
                                    final_skip_if.clone(),
                                ));
                        }
                    } else {
                        field_selections.insert(
                            response_key.to_string(),
                            ProjectionFieldSelection {
                                field_name,
                                response_key: response_key.clone(),
                                include_if: final_include_if.clone(),
                                skip_if: final_skip_if.clone(),
                                parent_type_conditions: parent_type_conditions.clone(),
                                selections: if field.selections.items.is_empty() {
                                    None
                                } else {
                                    Some(ProjectionFieldSelection::from_selection_set(
                                        &field.selections,
                                        field_type,
                                        None,
                                        schema_metadata,
                                        final_include_if.clone(),
                                        final_skip_if.clone(),
                                    ))
                                },
                                type_name: field_type.to_string(),
                                enum_values: schema_metadata.enum_values.get(field_type).cloned(),
                            },
                        );
                    }
                }
                SelectionItem::InlineFragment(inline_fragment) => {
                    let mut final_include_if: Option<String> = None;
                    let mut final_skip_if: Option<String> = None;
                    if include_if.is_some() {
                        final_include_if = include_if.clone();
                    }
                    if inline_fragment.include_if.is_some() {
                        final_include_if = inline_fragment.include_if.clone();
                    }
                    if skip_if.is_some() {
                        final_skip_if = skip_if.clone();
                    }
                    if inline_fragment.skip_if.is_some() {
                        final_skip_if = inline_fragment.skip_if.clone();
                    }
                    let mut parent_type_conditions = schema_metadata
                        .possible_types
                        .get_possible_types(&inline_fragment.type_condition)
                        .cloned();
                    if parent_type_conditions.is_none() {
                        let mut new_parent_type_conditions = HashSet::new();
                        new_parent_type_conditions.insert(inline_fragment.type_condition.clone());
                        parent_type_conditions = Some(new_parent_type_conditions);
                    }
                    let inline_fragment_selections = ProjectionFieldSelection::from_selection_set(
                        &inline_fragment.selections,
                        &inline_fragment.type_condition,
                        parent_type_conditions,
                        schema_metadata,
                        final_include_if,
                        final_skip_if,
                    );
                    for selection in inline_fragment_selections {
                        if let Some(existing_field) =
                            field_selections.get_mut(selection.response_key.as_str())
                        {
                            if let Some(existing_conditions) =
                                &mut existing_field.parent_type_conditions
                            {
                                if let Some(parent_type_conditions) =
                                    selection.parent_type_conditions
                                {
                                    existing_conditions.extend(parent_type_conditions.clone());
                                }
                            }
                            if let (Some(existing_field_include_if), Some(selection_include_if)) =
                                (&existing_field.include_if, &selection.include_if)
                            {
                                if existing_field_include_if != selection_include_if {
                                    existing_field.include_if = None;
                                }
                            }
                            if let (Some(existing_field_skip_if), Some(selection_skip_if)) =
                                (&existing_field.skip_if, &selection.skip_if)
                            {
                                if existing_field_skip_if != selection_skip_if {
                                    existing_field.skip_if = None;
                                }
                            }
                            if let Some(subselections) = selection.selections {
                                existing_field
                                    .selections
                                    .get_or_insert_with(Vec::new)
                                    .extend(subselections);
                            }
                        } else {
                            field_selections
                                .insert(selection.response_key.to_string(), selection.clone());
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
        field_selections.into_values().collect::<Vec<_>>()
    }
    pub fn from_operation(
        operation: &OperationDefinition,
        schema_metadata: &SchemaMetadata,
    ) -> Vec<ProjectionFieldSelection> {
        let root_type_name = match operation.operation_kind {
            Some(OperationKind::Query) => "Query",
            Some(OperationKind::Mutation) => "Mutation",
            Some(OperationKind::Subscription) => "Subscription",
            None => "Query",
        };
        ProjectionFieldSelection::from_selection_set(
            &operation.selection_set,
            root_type_name,
            None,
            schema_metadata,
            None,
            None,
        )
    }
}

#[instrument(level = "trace", skip_all)]
pub fn project_by_operation(
    data: &mut Value,
    errors: &mut Vec<GraphQLError>,
    extensions: &HashMap<String, Value>,
    selections: &Vec<ProjectionFieldSelection>,
    schema_metadata: &SchemaMetadata,
    variable_values: &Option<HashMap<String, Value>>,
) -> String {
    // We may want to remove it, but let's see.
    let mut buffer = String::with_capacity(4096);

    buffer.push('{');
    buffer.push('"');
    buffer.push_str("data");
    buffer.push('"');
    buffer.push(':');

    if let Some(data_map) = data.as_object_mut() {
        let mut first = true;
        project_selection_set_with_map(
            data_map,
            errors,
            selections,
            schema_metadata,
            variable_values,
            &mut buffer,
            &mut first, // Start with first as true to add the opening brace
        );
        if !first {
            buffer.push('}');
        } else {
            // If no selections were made, we should return an empty object
            buffer.push_str("{}");
        }
    } else {
        buffer.push_str("null");
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

    buffer.push('}');
    buffer
}

#[instrument(
    level = "trace",
    skip_all,
    fields(
        data = ?data
    )
)]
fn project_selection_set(
    data: &Value,
    errors: &mut Vec<GraphQLError>,
    selection: &ProjectionFieldSelection,
    schema_metadata: &SchemaMetadata,
    variable_values: &Option<HashMap<String, Value>>,
    buffer: &mut String,
) {
    match data {
        Value::Null => buffer.push_str("null"),
        Value::Bool(true) => buffer.push_str("true"),
        Value::Bool(false) => buffer.push_str("false"),
        Value::Number(num) => write!(buffer, "{}", num).unwrap(),
        Value::String(value) => {
            if let Some(enum_values) = &selection.enum_values {
                if !enum_values.contains(value) {
                    errors.push(GraphQLError {
                        message: "Value is not a valid enum value".to_string(),
                        locations: None,
                        path: None,
                        extensions: None,
                    });
                    buffer.push_str("null");
                    return;
                }
            }
            write_and_escape_string(buffer, value);
        }
        Value::Array(arr) => {
            buffer.push('[');
            let mut first = true;
            for item in arr.iter() {
                if !first {
                    buffer.push(',');
                }
                project_selection_set(
                    item,
                    errors,
                    selection,
                    schema_metadata,
                    variable_values,
                    buffer,
                );
                first = false;
            }
            buffer.push(']');
        }
        Value::Object(obj) => {
            let mut first = true;
            let is_projected = project_selection_set_with_map(
                obj,
                errors,
                selection.selections.as_ref().unwrap(),
                schema_metadata,
                variable_values,
                buffer,
                &mut first,
            );
            if !is_projected {
                buffer.push_str("null");
            } else
            // If first is mutated, it means we added "{"
            if !first {
                buffer.push('}');
            } else {
                // If first is still true, it means we didn't add anything, so we should just send an empty object
                buffer.push_str("{}");
            }
        }
    }
}

trait IncludeOrSkipByVariable {
    fn include_or_skip_by_variable(&self, variable_values: &Option<HashMap<String, Value>>)
        -> bool;
}

impl IncludeOrSkipByVariable for ProjectionFieldSelection {
    fn include_or_skip_by_variable(
        &self,
        variable_values: &Option<HashMap<String, Value>>,
    ) -> bool {
        if let Some(skip_variable) = &self.skip_if {
            if let Some(variable_value) = variable_values
                .as_ref()
                .and_then(|vars| vars.get(skip_variable))
            {
                if variable_value == &Value::Bool(true) {
                    return false; // Skip this field if the variable is true
                }
            }
            return true;
        }
        if let Some(include_variable) = &self.include_if {
            if let Some(variable_value) = variable_values
                .as_ref()
                .and_then(|vars| vars.get(include_variable))
            {
                if variable_value == &Value::Bool(true) {
                    return true; // Skip this field if the variable is not true
                }
            }
            return false;
        }
        true
    }
}

#[instrument(
    level = "trace",
    skip_all,
    fields(
        obj = ?obj
    )
)]
// TODO: simplfy args
#[allow(clippy::too_many_arguments)]
fn project_selection_set_with_map(
    obj: &Map<String, Value>,
    errors: &mut Vec<GraphQLError>,
    selections: &Vec<ProjectionFieldSelection>,
    schema_metadata: &SchemaMetadata,
    variable_values: &Option<HashMap<String, Value>>,
    buffer: &mut String,
    first: &mut bool,
) -> bool {
    let type_name = match obj.get(TYPENAME_FIELD) {
        Some(Value::String(type_name)) => Some(type_name),
        _ => None,
    };
    for selection in selections {
        if !selection.include_or_skip_by_variable(variable_values) {
            // If the selection is not included by variable, skip it
            continue;
        }
        if let Some(parent_conditions) = &selection.parent_type_conditions {
            if let Some(type_name) = type_name {
                if !parent_conditions.contains(type_name) {
                    // If the type name is not in the parent type conditions, skip it
                    continue;
                }
            }
        }

        if *first {
            buffer.push('{');
        } else {
            buffer.push(',');
        }
        *first = false;

        if selection.field_name == TYPENAME_FIELD {
            buffer.push('"');
            buffer.push_str(&selection.response_key);
            buffer.push_str("\":\"");
            let type_name = obj
                .get(TYPENAME_FIELD)
                .and_then(|v| v.as_str())
                .unwrap_or(&selection.type_name);
            buffer.push_str(type_name);
            buffer.push('"');
            continue;
        }

        buffer.push('"');
        buffer.push_str(&selection.response_key);
        buffer.push_str("\":");

        let field_val = if selection.field_name == "__schema" && selection.type_name == "Query" {
            Some(&schema_metadata.introspection_schema_root_json)
        } else {
            obj.get(selection.response_key.as_str())
        };

        if let Some(field_val) = field_val {
            project_selection_set(
                field_val,
                errors,
                selection,
                schema_metadata,
                variable_values,
                buffer,
            );
        } else {
            // If the field is not found in the object, set it to Null
            buffer.push_str("null");
            continue;
        }
    }
    true
}
