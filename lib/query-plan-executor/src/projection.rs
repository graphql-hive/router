use std::collections::BTreeMap;

use query_planner::{
    ast::{
        operation::OperationDefinition, selection_item::SelectionItem, selection_set::SelectionSet,
    },
    state::supergraph_state::OperationKind,
};
use serde_json::{Map, Value};
use tracing::{instrument, warn};

use crate::{
    deep_merge::DeepMerge,
    execution_result::{ExecutionResult, GraphQLError},
    schema_metadata::{EntitySatisfiesTypeCondition, SchemaMetadata},
    TYPENAME_FIELD,
};

pub trait SelectionSetProjection {
    fn project_for_requires(&self, parent: &Value, schema_metadata: &SchemaMetadata) -> Value;
}

impl SelectionSetProjection for SelectionSet {
    fn project_for_requires(&self, entity: &Value, schema_metadata: &SchemaMetadata) -> Value {
        if self.is_empty() {
            return entity.to_owned(); // No selections to project, return the entity as is
        }
        match entity {
            Value::Null => Value::Null,
            Value::Array(entity_array) => Value::Array(
                entity_array
                    .iter()
                    .map(|item| self.project_for_requires(item, schema_metadata))
                    .collect(),
            ),
            Value::Object(entity_obj) => {
                let mut result_map = Map::with_capacity(entity_obj.len().max(self.items.len()));
                for requires_selection in &self.items {
                    match &requires_selection {
                        SelectionItem::Field(requires_selection) => {
                            let field_name = &requires_selection.name;
                            let response_key = requires_selection.selection_identifier();
                            let original = entity_obj
                                .get(field_name)
                                .unwrap_or(entity_obj.get(response_key).unwrap_or(&Value::Null));
                            let projected_value: Value = requires_selection
                                .selections
                                .project_for_requires(original, schema_metadata);
                            if !projected_value.is_null() {
                                result_map.insert(response_key.to_string(), projected_value);
                            }
                        }
                        SelectionItem::InlineFragment(requires_selection) => {
                            let type_name = match entity_obj.get(TYPENAME_FIELD) {
                                Some(Value::String(type_name)) => type_name,
                                _ => requires_selection.type_condition.as_str(),
                            };
                            if schema_metadata.entity_satisfies_type_condition(
                                type_name,
                                &requires_selection.type_condition,
                            ) {
                                let projected = requires_selection
                                    .selections
                                    .project_for_requires(entity, schema_metadata);
                                // Merge the projected value into the result
                                if let Value::Object(projected_map) = projected {
                                    result_map.deep_merge(projected_map);
                                }
                                // If the projected value is not an object, it will be ignored
                            }
                        }
                    }
                }
                if (result_map.is_empty())
                    || (result_map.len() == 1 && result_map.contains_key(TYPENAME_FIELD))
                {
                    Value::Null
                } else {
                    Value::Object(result_map)
                }
            }
            Value::Bool(bool) => Value::Bool(*bool),
            Value::Number(num) => Value::Number(num.to_owned()),
            Value::String(string) => Value::String(string.to_string()),
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
    variable_values: &Option<BTreeMap<String, Value>>,
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
                if schema_metadata
                    .entity_satisfies_type_condition(&type_name, &inline_fragment.type_condition)
                {
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
    variables: &Option<BTreeMap<String, Value>>,
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
            "\"".to_string() + value + "\"" // Return the string value wrapped in quotes
        }
        Value::Array(arr) => {
            let items = arr
                .iter_mut()
                .map(|item| {
                    project_selection_set(
                        item,
                        errors,
                        selection_set,
                        type_name,
                        schema_metadata,
                        variables,
                    )
                })
                .collect::<Vec<_>>();
            "[".to_string() + &items.join(",") + "]"
        }
        Value::Object(obj) => {
            let items = project_selection_set_with_map(
                obj,
                errors,
                selection_set,
                type_name,
                schema_metadata,
                variables,
            );
            "{".to_string() + &items.join(",") + "}"
        }
    }
}

#[instrument(level = "trace", skip_all)]
pub fn project_by_operation(
    result: ExecutionResult,
    operation: &OperationDefinition,
    schema_metadata: &SchemaMetadata,
    variables: &Option<BTreeMap<String, Value>>,
) -> String {
    let root_type_name = match operation.operation_kind {
        Some(OperationKind::Query) => "Query",
        Some(OperationKind::Mutation) => "Mutation",
        Some(OperationKind::Subscription) => "Subscription",
        None => "Query",
    };
    let mut items = Vec::with_capacity(3);
    let mut errors = result.errors.unwrap_or_default();
    if let Some(mut data) = result.data {
        // Project the data based on the selection set
        let data_str = project_selection_set(
            &mut data,
            &mut errors,
            &operation.selection_set,
            root_type_name,
            schema_metadata,
            variables,
        );
        let data_entry = "\"data\":".to_string() + &data_str;
        items.push(data_entry);
    }

    if !errors.is_empty() {
        let errors_entry = "\"errors\":".to_string() + &serde_json::to_string(&errors).unwrap();
        items.push(errors_entry);
    }
    if result.extensions.is_some() {
        let extensions = result.extensions.unwrap_or_default();
        let extensions_entry =
            "\"extensions\":".to_string() + &serde_json::to_string(&extensions).unwrap();
        items.push(extensions_entry);
    }

    let entries = items.join(",");
    "{".to_string() + &entries + "}"
}
