use std::collections::HashMap;

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
    // Project the data based on the selection set
    let data = project_selection_set(
        data,
        errors,
        &operation.selection_set,
        root_type_name,
        schema_metadata,
        variable_values,
    );
    let mut items = "{\"data\":".to_string() + &data;
    if !errors.is_empty() {
        let errors_entry = ",\"errors\":".to_string() + &serde_json::to_string(&errors).unwrap();
        items.push_str(&errors_entry);
    }
    if !extensions.is_empty() {
        let extensions_entry =
            ",\"extensions\":".to_string() + &serde_json::to_string(&extensions).unwrap();
        items.push_str(&extensions_entry);
    }

    items.push('}');

    items
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
) -> String {
    match data {
        Value::Null => "null".to_string(),
        Value::Bool(true) => "true".to_string(),
        Value::Bool(false) => "false".to_string(),
        Value::Number(num) => num.to_string(),
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
                    return "null".to_string(); // Set data to Null if the value is not valid
                }
            }
            serde_json::to_string(value).unwrap() // Return the string value wrapped in quotes
        }
        Value::Array(arr) => {
            let items: Vec<String> = arr
                .iter_mut()
                .map(|item| {
                    project_selection_set(
                        item,
                        errors,
                        selection_set,
                        type_name,
                        schema_metadata,
                        variable_values,
                    )
                })
                .collect();
            "[".to_string() + &items.join(",") + "]"
        }
        Value::Object(obj) => {
            let items = project_selection_set_with_map(
                obj,
                errors,
                selection_set,
                type_name,
                schema_metadata,
                variable_values,
            );
            "{".to_string() + &items.join(",") + "}"
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
fn project_selection_set_with_map(
    obj: &mut Map<String, Value>,
    errors: &mut Vec<GraphQLError>,
    selection_set: &SelectionSet,
    type_name: &str,
    schema_metadata: &SchemaMetadata,
    variable_values: &Option<HashMap<String, Value>>,
) -> Vec<String> {
    let type_name = match obj.get(TYPENAME_FIELD) {
        Some(Value::String(type_name)) => type_name,
        _ => type_name,
    }
    .to_string();
    let field_map = schema_metadata.type_fields.get(&type_name);
    let mut items = vec![];
    for selection in &selection_set.items {
        match selection {
            SelectionItem::Field(field) => {
                // Get the type fields for the current type
                // Type is not found in the schema
                if field_map.is_none() {
                    // It won't reach here already, as the selection should be validated before projection
                    warn!("Type {} not found. Skipping projection.", type_name);
                    continue;
                }
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
                let response_key = field.alias.as_ref().unwrap_or(&field.name).to_string();
                if field.name == TYPENAME_FIELD {
                    items.push("\"".to_string() + &response_key + "\":\"" + &type_name + "\"");
                    continue;
                }
                let field_map = field_map.unwrap();
                let field_type = field_map.get(&field.name);
                if field.name == "__schema" && type_name == "Query" {
                    obj.insert(
                        response_key.to_string(),
                        schema_metadata.introspection_schema_root_json.clone(),
                    );
                }
                let field_val = obj.get_mut(&response_key);
                match (field_type, field_val) {
                    (Some(field_type), Some(field_val)) => {
                        let projected = project_selection_set(
                            field_val,
                            errors,
                            &field.selections,
                            field_type,
                            schema_metadata,
                            variable_values,
                        );
                        items.push("\"".to_string() + &response_key + "\":" + &projected);
                    }
                    (Some(_field_type), None) => {
                        // If the field is not found in the object, set it to Null
                        items.push("\"".to_string() + &response_key + "\":null");
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
                    let projected = project_selection_set_with_map(
                        obj,
                        errors,
                        &inline_fragment.selections,
                        &type_name,
                        schema_metadata,
                        variable_values,
                    );
                    items.extend(projected);
                }
            }
        }
    }
    items
}
