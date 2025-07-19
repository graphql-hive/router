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
    parent_type_conditions: HashSet<String>,
    enum_values: Option<HashSet<String>>,
    selections: Option<Vec<ProjectionFieldSelection>>,
}

impl ProjectionFieldSelection {
    pub fn from_selection_set(
        selection_set: &SelectionSet,
        parent_type_name: &str,
        parent_type_conditions: HashSet<String>,
        schema_metadata: &SchemaMetadata,
        include_if: Option<String>,
        skip_if: Option<String>,
    ) -> Option<Vec<ProjectionFieldSelection>> {
        let mut field_selections: IndexMap<String, ProjectionFieldSelection> = IndexMap::new();
        for selection_item in &selection_set.items {
            match selection_item {
                SelectionItem::Field(field) => {
                    let field_name = field.name.clone();
                    let response_key = field.alias.as_ref().unwrap_or(&field.name);
                    let field_type = if field_name == TYPENAME_FIELD {
                        "String"
                    } else {
                        let field_map = match schema_metadata.type_fields.get(parent_type_name) {
                            Some(fields) => fields,
                            None => {
                                warn!(
                                    "No fields found for type {} in schema metadata.",
                                    parent_type_name
                                );
                                return None;
                            }
                        };
                        match field_map.get(&field_name) {
                            Some(field_type) => field_type,
                            None => {
                                warn!(
                                    "Field {} not found in type {} in schema metadata.",
                                    field_name, parent_type_name
                                );
                                continue;
                            }
                        }
                    };
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
                        existing_field
                            .parent_type_conditions
                            .extend(parent_type_conditions.clone());
                        if existing_field.include_if != final_include_if {
                            existing_field.include_if = None;
                        }
                        if existing_field.skip_if != final_skip_if {
                            existing_field.skip_if = None;
                        }
                        if field.selections.items.is_empty() {
                            existing_field.selections = None;
                        } else if let Some(new_selections) = {
                            let field_type_conditions = schema_metadata
                                .possible_types
                                .get_possible_types(field_type);
                            ProjectionFieldSelection::from_selection_set(
                                &field.selections,
                                field_type,
                                field_type_conditions,
                                schema_metadata,
                                final_include_if.clone(),
                                final_skip_if.clone(),
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
                        let field_type_conditions = schema_metadata
                            .possible_types
                            .get_possible_types(field_type);
                        field_selections.insert(
                            response_key.to_string(),
                            ProjectionFieldSelection {
                                field_name,
                                response_key: response_key.clone(),
                                include_if: final_include_if.clone(),
                                skip_if: final_skip_if.clone(),
                                parent_type_conditions: parent_type_conditions.clone(),
                                selections: ProjectionFieldSelection::from_selection_set(
                                    &field.selections,
                                    field_type,
                                    field_type_conditions,
                                    schema_metadata,
                                    final_include_if.clone(),
                                    final_skip_if.clone(),
                                ),
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
                    let parent_type_conditions = schema_metadata
                        .possible_types
                        .get_possible_types(&inline_fragment.type_condition);
                    if let Some(inline_fragment_selections) =
                        ProjectionFieldSelection::from_selection_set(
                            &inline_fragment.selections,
                            &inline_fragment.type_condition,
                            parent_type_conditions,
                            schema_metadata,
                            final_include_if,
                            final_skip_if,
                        )
                    {
                        for selection in inline_fragment_selections {
                            if let Some(existing_field) =
                                field_selections.get_mut(selection.response_key.as_str())
                            {
                                existing_field
                                    .parent_type_conditions
                                    .extend(selection.parent_type_conditions);
                                if existing_field.include_if != selection.include_if {
                                    existing_field.include_if = None;
                                }
                                if existing_field.skip_if != selection.skip_if {
                                    existing_field.skip_if = None;
                                }
                                if let Some(subselections) = selection.selections {
                                    if let Some(existing_selections) =
                                        &mut existing_field.selections
                                    {
                                        existing_selections.extend(subselections);
                                    } else {
                                        existing_field.selections = Some(subselections);
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
    pub fn from_operation(
        operation: &OperationDefinition,
        schema_metadata: &SchemaMetadata,
    ) -> (&'static str, Vec<ProjectionFieldSelection>) {
        let root_type_name = match operation.operation_kind {
            Some(OperationKind::Query) => "Query",
            Some(OperationKind::Mutation) => "Mutation",
            Some(OperationKind::Subscription) => "Subscription",
            None => "Query",
        };
        let type_conditions = HashSet::from([root_type_name.to_string()]);
        (
            root_type_name,
            ProjectionFieldSelection::from_selection_set(
                &operation.selection_set,
                root_type_name,
                type_conditions,
                schema_metadata,
                None,
                None,
            )
            .unwrap(),
        )
    }
}

#[instrument(level = "trace", skip_all)]
pub fn project_by_operation(
    data: &mut Value,
    errors: &mut Vec<GraphQLError>,
    extensions: &HashMap<String, Value>,
    operation_type_name: &str,
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
            operation_type_name,
            &mut buffer,
            &mut first, // Start with first as true to add the opening brace
        );
        if !first {
            buffer.push('}');
        } else {
            // If no selections were made, we should return an empty object
            buffer.push_str("{}");
        }
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
            match selection.selections.as_ref() {
                Some(selections) => {
                    let type_name = obj
                        .get(TYPENAME_FIELD)
                        .and_then(|v| v.as_str())
                        .unwrap_or(&selection.type_name);
                    if !schema_metadata
                        .possible_types
                        .entity_satisfies_type_condition(type_name, &selection.type_name)
                    {
                        buffer.push_str("null");
                    } else {
                        let mut first = true;
                        project_selection_set_with_map(
                            obj,
                            errors,
                            selections,
                            schema_metadata,
                            variable_values,
                            type_name,
                            buffer,
                            &mut first,
                        );
                        if !first {
                            buffer.push('}');
                        } else {
                            // If no selections were made, we should return an empty object
                            buffer.push_str("{}");
                        }
                    }
                }
                None => {
                    // If the selection set is not projected, we should return null
                    buffer.push_str("null");
                }
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
    parent_type_name: &str,
    buffer: &mut String,
    first: &mut bool,
) {
    for selection in selections {
        if !selection.include_or_skip_by_variable(variable_values) {
            // If the selection is not included by variable, skip it
            continue;
        }
        if !selection.parent_type_conditions.contains(parent_type_name) {
            // If the type name is not in the parent type conditions, skip it
            continue;
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
            buffer.push_str(parent_type_name);
            buffer.push('"');
            continue;
        }

        buffer.push('"');
        buffer.push_str(&selection.response_key);
        buffer.push_str("\":");

        let field_val = if selection.field_name == "__schema" && parent_type_name == "Query" {
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
}
