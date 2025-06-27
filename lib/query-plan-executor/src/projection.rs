use std::collections::HashMap;
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
    entity_satisfies_type_condition, schema_metadata::SchemaMetadata, GraphQLError, TYPENAME_FIELD,
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

    write!(buffer, "{{\"data\":").unwrap();
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
    data: &mut Value,
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
                    *data = Value::Null;
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
            // Use serde_json to handle proper string escaping
            write!(buffer, "{}", serde_json::to_string(value).unwrap()).unwrap();
        }
        Value::Array(arr) => {
            buffer.push('[');
            let mut first = true;
            for item in arr.iter_mut() {
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
            buffer.push('{');
            let mut first = true;
            project_selection_set_with_map(
                obj,
                errors,
                selection_set,
                type_name,
                schema_metadata,
                variable_values,
                buffer,
                &mut first,
            );
            buffer.push('}');
        }
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
    obj: &mut Map<String, Value>,
    errors: &mut Vec<GraphQLError>,
    selection_set: &SelectionSet,
    type_name: &str,
    schema_metadata: &SchemaMetadata,
    variable_values: &Option<HashMap<String, Value>>,
    buffer: &mut String,
    first: &mut bool,
) {
    let type_name = match obj.get(TYPENAME_FIELD) {
        Some(Value::String(type_name)) => type_name,
        _ => type_name,
    }
    .to_string();
    let field_map = schema_metadata.type_fields.get(&type_name);

    for selection in &selection_set.items {
        match selection {
            SelectionItem::Field(field) => {
                if let Some(ref skip_variable) = field.skip_if {
                    let variable_value = variable_values
                        .as_ref()
                        .and_then(|vars| vars.get(skip_variable));
                    if variable_value == Some(&Value::Bool(true)) {
                        continue; // Skip this field if the variable is true
                    }
                }
                if let Some(ref include_variable) = field.include_if {
                    let variable_value = variable_values
                        .as_ref()
                        .and_then(|vars| vars.get(include_variable));
                    if variable_value != Some(&Value::Bool(true)) {
                        continue; // Skip this field if the variable is not true
                    }
                }

                let response_key = field.alias.as_ref().unwrap_or(&field.name);

                if !*first {
                    buffer.push(',');
                }
                *first = false;

                if field.name == TYPENAME_FIELD {
                    write!(buffer, "\"{}\":\"{}\"", response_key, type_name).unwrap();
                    continue;
                }

                write!(buffer, "\"{}\":", response_key).unwrap();

                let field_map = field_map.unwrap();
                let field_type = field_map.get(&field.name);

                if field.name == "__schema" && type_name == "Query" {
                    obj.insert(
                        response_key.to_string(),
                        schema_metadata.introspection_schema_root_json.clone(),
                    );
                }

                let field_val = obj.get_mut(response_key);

                match (field_type, field_val) {
                    (Some(field_type), Some(field_val)) => {
                        project_selection_set(
                            field_val,
                            errors,
                            &field.selections,
                            field_type,
                            schema_metadata,
                            variable_values,
                            buffer,
                        );
                    }
                    (Some(_field_type), None) => {
                        // If the field is not found in the object, set it to Null
                        buffer.push_str("null");
                    }
                    (None, _) => {
                        // It won't reach here already, as the selection should be validated before projection
                        warn!(
                            "Field {} not found in type {}. Skipping projection.",
                            field.name, type_name
                        );
                    }
                }
            }
            SelectionItem::InlineFragment(inline_fragment) => {
                if entity_satisfies_type_condition(
                    &schema_metadata.possible_types,
                    &type_name,
                    &inline_fragment.type_condition,
                ) {
                    project_selection_set_with_map(
                        obj,
                        errors,
                        &inline_fragment.selections,
                        &type_name,
                        schema_metadata,
                        variable_values,
                        buffer,
                        first,
                    );
                }
            }
        }
    }
}
