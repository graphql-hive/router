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
    execution_result::GraphQLError,
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
    variable_values: &Option<Map<String, Value>>,
) -> Option<Map<String, Value>> {
    let type_name = match obj.get(TYPENAME_FIELD) {
        Some(Value::String(type_name)) => type_name,
        _ => type_name,
    }
    .to_string();
    let mut new_obj = Map::with_capacity(obj.len().max(selection_set.items.len()));
    let field_map = schema_metadata.type_fields.get(&type_name);
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
                    new_obj.insert(response_key, Value::String(type_name.to_string()));
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
                        match field_val {
                            Value::Object(field_val_map) => {
                                let new_field_val_map = project_selection_set_with_map(
                                    field_val_map,
                                    errors,
                                    &field.selections,
                                    field_type,
                                    schema_metadata,
                                    variable_values,
                                );
                                match new_field_val_map {
                                    Some(new_field_val_map) => {
                                        // If the field is an object, merge the projected values
                                        new_obj
                                            .insert(response_key, Value::Object(new_field_val_map));
                                    }
                                    None => {
                                        new_obj.insert(response_key, Value::Null);
                                    }
                                }
                            }
                            field_val => {
                                project_selection_set(
                                    field_val,
                                    errors,
                                    &field.selections,
                                    field_type,
                                    schema_metadata,
                                    variable_values,
                                );
                                new_obj.insert(
                                    response_key,
                                    field_val.clone(), // Clone the value to insert
                                );
                            }
                        }
                    }
                    (Some(_field_type), None) => {
                        // If the field is not found in the object, set it to Null
                        new_obj.insert(response_key, Value::Null);
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
                    let sub_new_obj = project_selection_set_with_map(
                        obj,
                        errors,
                        &inline_fragment.selections,
                        &type_name,
                        schema_metadata,
                        variable_values,
                    );
                    if let Some(sub_new_obj) = sub_new_obj {
                        // If the inline fragment projection returns a new object, merge it
                        new_obj.deep_merge(sub_new_obj);
                    } else {
                        // If the inline fragment projection returns None, skip it
                        continue;
                    }
                }
            }
        }
    }
    Some(new_obj)
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
    variable_values: &Option<Map<String, Value>>,
) {
    match data {
        Value::Null => {
            // If data is Null, no need to project further
        }
        Value::String(value) => {
            if let Some(enum_values) = schema_metadata.enum_values.get(type_name) {
                if !enum_values.contains(value) {
                    // If the value is not a valid enum value, add an error
                    // and set data to Null
                    *data = Value::Null; // Set data to Null if the value is not valid
                    errors.push(GraphQLError {
                        message: format!(
                            "Value is not a valid enum value for type '{}'",
                            type_name
                        ),
                        locations: None,
                        path: None,
                        extensions: None,
                    });
                }
            } // No further processing needed for strings
        }
        Value::Array(arr) => {
            // If data is an array, project each item in the array
            for item in arr {
                project_selection_set(
                    item,
                    errors,
                    selection_set,
                    type_name,
                    schema_metadata,
                    variable_values,
                );
            } // No further processing needed for arrays
        }
        Value::Object(obj) => {
            match project_selection_set_with_map(
                obj,
                errors,
                selection_set,
                type_name,
                schema_metadata,
                variable_values,
            ) {
                Some(new_obj) => {
                    // If the projection returns a new object, replace the old one
                    *obj = new_obj;
                }
                None => {
                    // If the projection returns None, set data to Null
                    *data = Value::Null;
                }
            }
        }
        _ => {}
    }
}

#[instrument(level = "trace", skip_all)]
pub fn project_data_by_operation(
    data: &mut Value,
    errors: &mut Vec<GraphQLError>,
    operation: &OperationDefinition,
    schema_metadata: &SchemaMetadata,
    variable_values: &Option<Map<String, Value>>,
) {
    let root_type_name = match operation.operation_kind {
        Some(OperationKind::Query) => "Query",
        Some(OperationKind::Mutation) => "Mutation",
        Some(OperationKind::Subscription) => "Subscription",
        None => "Query",
    };
    // Project the data based on the selection set
    project_selection_set(
        data,
        errors,
        &operation.selection_set,
        root_type_name,
        schema_metadata,
        variable_values,
    )
}
