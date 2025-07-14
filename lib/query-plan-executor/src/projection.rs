use hashbrown::HashMap;
use std::fmt::Write;

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

#[instrument(level = "trace", skip_all)]
pub fn project_by_operation(
    data: &mut Value,
    errors: &mut Vec<GraphQLError>,
    extensions: &HashMap<String, Value>,
    operation: &OperationDefinition,
    schema_metadata: &SchemaMetadata,
    variable_values: &Option<HashMap<String, Value>>,
) -> String {
    let root_type_name = match operation.operation_kind {
        Some(OperationKind::Query) => "Query",
        Some(OperationKind::Mutation) => "Mutation",
        Some(OperationKind::Subscription) => "Subscription",
        None => "Query",
    };

    // We may want to remove it, but let's see.
    let mut buffer = String::with_capacity(4096);

    buffer.push('{');
    buffer.push('"');
    buffer.push_str("data");
    buffer.push('"');
    buffer.push(':');

    project_selection_set(
        data,
        errors,
        &operation.selection_set,
        root_type_name,
        schema_metadata,
        variable_values,
        &mut buffer,
    );

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
        type_name = %type_name,
        selection_set = ?selection_set.items.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        data = ?data
    )
)]
fn project_selection_set(
    data: &Value,
    errors: &mut Vec<GraphQLError>,
    selection_set: &SelectionSet,
    type_name: &str,
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
            if let Some(enum_values) = schema_metadata.enum_values.get(type_name) {
                if !enum_values.contains(value) {
                    errors.push(GraphQLError {
                        message: format!(
                            "Value is not a valid enum value for type '{}'",
                            type_name
                        ),
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
                    selection_set,
                    type_name,
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
                selection_set,
                type_name,
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

impl IncludeOrSkipByVariable for SelectionItem {
    fn include_or_skip_by_variable(
        &self,
        variable_values: &Option<HashMap<String, Value>>,
    ) -> bool {
        if let Some(skip_variable) = match self {
            SelectionItem::Field(field) => field.skip_if.as_ref(),
            SelectionItem::InlineFragment(inline_fragment) => inline_fragment.skip_if.as_ref(),
            SelectionItem::FragmentSpread(_) => {
                // Fragment spreads should not exist in the final response projection.
                unreachable!("Fragment spreads should not exist in the final response projection.")
            }
        } {
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
        if let Some(include_variable) = match self {
            SelectionItem::Field(field) => field.include_if.as_ref(),
            SelectionItem::InlineFragment(inline_fragment) => inline_fragment.include_if.as_ref(),
            SelectionItem::FragmentSpread(_) => {
                // Fragment spreads should not exist in the final response projection.
                unreachable!("Fragment spreads should not exist in the final response projection.")
            }
        } {
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
        type_name = %type_name,
        selection_set = ?selection_set.items.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        obj = ?obj
    )
)]
// TODO: simplfy args
#[allow(clippy::too_many_arguments)]
fn project_selection_set_with_map(
    obj: &Map<String, Value>,
    errors: &mut Vec<GraphQLError>,
    selection_set: &SelectionSet,
    type_name: &str,
    schema_metadata: &SchemaMetadata,
    variable_values: &Option<HashMap<String, Value>>,
    buffer: &mut String,
    first: &mut bool,
) -> bool {
    let type_name = match obj.get(TYPENAME_FIELD) {
        Some(Value::String(type_name)) => type_name,
        _ => type_name,
    };
    let field_map = match schema_metadata.type_fields.get(type_name) {
        Some(field_map) => field_map,
        None => {
            // If the type is not found, we can't project anything
            warn!(
                "Type {} not found in schema metadata. Skipping projection.",
                type_name
            );
            return false;
        }
    };

    for selection in &selection_set.items {
        if !selection.include_or_skip_by_variable(variable_values) {
            // If the selection is not included by variable, skip it
            continue;
        }
        match selection {
            SelectionItem::Field(field) => {
                let response_key = field.alias.as_ref().unwrap_or(&field.name);

                if *first {
                    buffer.push('{');
                } else {
                    buffer.push(',');
                }
                *first = false;

                if field.name == TYPENAME_FIELD {
                    buffer.push('"');
                    buffer.push_str(response_key);
                    buffer.push_str("\":\"");
                    buffer.push_str(type_name);
                    buffer.push('"');
                    continue;
                }

                buffer.push('"');
                buffer.push_str(response_key);
                buffer.push_str("\":");

                let field_type: &str = field_map
                    .get(&field.name)
                    .map(|s| s.as_str())
                    .unwrap_or("Any");

                let field_val = if field.name == "__schema" && type_name == "Query" {
                    Some(&schema_metadata.introspection_schema_root_json)
                } else {
                    obj.get(response_key)
                };

                if let Some(field_val) = field_val {
                    project_selection_set(
                        field_val,
                        errors,
                        &field.selections,
                        field_type,
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
            SelectionItem::InlineFragment(inline_fragment) => {
                if schema_metadata
                    .possible_types
                    .entity_satisfies_type_condition(type_name, &inline_fragment.type_condition)
                {
                    project_selection_set_with_map(
                        obj,
                        errors,
                        &inline_fragment.selections,
                        type_name,
                        schema_metadata,
                        variable_values,
                        buffer,
                        first,
                    );
                }
            }
            SelectionItem::FragmentSpread(_name_ref) => {
                // We only minify the queries to subgraphs, so we never have fragment spreads here.
                // In this projection, we expect only inline fragments and fields
                // as it's the query produced by the ast normalization process.
                unreachable!("Fragment spreads should not exist in the final response projection.");
            }
        }
    }
    true
}
