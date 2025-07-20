use std::collections::{HashMap, HashSet};
use std::fmt::Write;

use indexmap::IndexMap;
use query_planner::ast::selection_set::FieldSelection;
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
pub struct FieldProjectionPlan {
    field_name: String,
    response_key: String,
    conditions: FieldProjectionConditions,
    selections: Option<Vec<FieldProjectionPlan>>,
}

#[derive(Debug, Clone)]
pub struct FieldProjectionConditions {
    type_name: String,
    field_type_conditions: Option<HashSet<String>>,
    skip_if_variables: Option<HashSet<String>>,
    include_if_variables: Option<HashSet<String>>,
    parent_type_conditions: Option<HashSet<String>>,
    enum_value_conditions: Option<HashSet<String>>,
}

impl FieldProjectionConditions {
    // Returns false if the projection is not possible
    pub fn add_skip_if_variable(&mut self, variable: &str) -> bool {
        if let Some(include_if_variables) = &mut self.include_if_variables {
            if include_if_variables.contains(variable) {
                return false;
            }
        }
        if let Some(skip_if_variables) = &mut self.skip_if_variables {
            skip_if_variables.insert(variable.to_owned());
        } else {
            self.skip_if_variables = Some(HashSet::from([variable.to_owned()]));
        }
        true
    }
    // Returns false if the projection is not possible
    pub fn add_include_if_variable(&mut self, variable: &str) -> bool {
        if let Some(skip_if_variables) = &mut self.skip_if_variables {
            if skip_if_variables.contains(variable) {
                return false;
            }
        }
        if let Some(include_if_variables) = &mut self.include_if_variables {
            include_if_variables.insert(variable.to_owned());
        } else {
            self.include_if_variables = Some(HashSet::from([variable.to_owned()]));
        }
        true
    }
    pub fn add_parent_type_condition(&mut self, type_name: &str, schema_metadata: &SchemaMetadata) {
        let possible_types = schema_metadata.possible_types.get_possible_types(type_name);
        if let Some(parent_conditions) = &mut self.parent_type_conditions {
            parent_conditions.extend(possible_types);
        } else {
            self.parent_type_conditions = Some(possible_types);
        }
    }
    pub fn add_field_type_condition(&mut self, type_name: &str, schema_metadata: &SchemaMetadata) {
        let possible_types = schema_metadata.possible_types.get_possible_types(type_name);
        if let Some(field_type_conditions) = &mut self.field_type_conditions {
            field_type_conditions.extend(possible_types);
        } else {
            self.field_type_conditions = Some(HashSet::from_iter(possible_types));
        }
    }
    pub fn extend(&mut self, other: &FieldProjectionConditions) {
        if let Some(skip_if_variables) = &other.skip_if_variables {
            if let Some(self_skip_if_variables) = &mut self.skip_if_variables {
                self_skip_if_variables.extend(skip_if_variables.clone());
            } else {
                self.skip_if_variables = Some(skip_if_variables.clone());
            }
        }
        if let Some(include_if_variables) = &other.include_if_variables {
            if let Some(self_include_if_variables) = &mut self.include_if_variables {
                self_include_if_variables.extend(include_if_variables.clone());
            } else {
                self.include_if_variables = Some(include_if_variables.clone());
            }
        }
        if let Some(parent_type_conditions) = &other.parent_type_conditions {
            if let Some(self_parent_type_conditions) = &mut self.parent_type_conditions {
                self_parent_type_conditions.extend(parent_type_conditions.clone());
            } else {
                self.parent_type_conditions = Some(parent_type_conditions.clone());
            }
        }
        if let Some(field_type_conditions) = &other.field_type_conditions {
            if let Some(self_field_type_conditions) = &mut self.field_type_conditions {
                self_field_type_conditions.extend(field_type_conditions.clone());
            } else {
                self.field_type_conditions = Some(field_type_conditions.clone());
            }
        }
        if let Some(enum_value_conditions) = &other.enum_value_conditions {
            if let Some(self_enum_value_conditions) = &mut self.enum_value_conditions {
                self_enum_value_conditions.extend(enum_value_conditions.clone());
            } else {
                self.enum_value_conditions = Some(enum_value_conditions.clone());
            }
        }
    }
    pub fn clone_for_parent_type(
        &self,
        parent_type: &str,
        schema_metadata: &SchemaMetadata,
        include_if: Option<&String>,
        skip_if: Option<&String>,
        // Return None if the projection is not possible
    ) -> Option<FieldProjectionConditions> {
        let mut new_condition = FieldProjectionConditions {
            type_name: parent_type.to_owned(),
            skip_if_variables: self.skip_if_variables.clone(),
            include_if_variables: self.include_if_variables.clone(),
            parent_type_conditions: Some(
                schema_metadata
                    .possible_types
                    .get_possible_types(parent_type),
            ),
            field_type_conditions: None,
            enum_value_conditions: None,
        };
        if let Some(include_if) = include_if {
            // Skip the projection if it is not possible
            if !new_condition.add_include_if_variable(include_if) {
                return None;
            }
        }
        if let Some(skip_if) = skip_if {
            // Skip the projection if it is not possible
            if !new_condition.add_skip_if_variable(skip_if) {
                return None;
            }
        }
        Some(new_condition)
    }
    pub fn for_field_selection(
        &self,
        field_type: &str,
        field_selection: &FieldSelection,
        schema_metadata: &SchemaMetadata,
        // Return None if the projection is not possible
    ) -> Option<FieldProjectionConditions> {
        let mut new_condition = FieldProjectionConditions {
            type_name: field_type.to_string(),
            skip_if_variables: self.skip_if_variables.clone(),
            include_if_variables: self.include_if_variables.clone(),
            parent_type_conditions: self.parent_type_conditions.clone(),
            field_type_conditions: Some(
                schema_metadata
                    .possible_types
                    .get_possible_types(field_type),
            ),
            enum_value_conditions: schema_metadata.enum_values.get(field_type).cloned(),
        };
        if let Some(include_if) = &field_selection.include_if {
            // Skip the projection if it is not possible
            if !new_condition.add_include_if_variable(include_if) {
                return None;
            }
        }
        if let Some(skip_if) = &field_selection.skip_if {
            // Skip the projection if it is not possible
            if !new_condition.add_skip_if_variable(skip_if) {
                return None;
            }
        }
        Some(new_condition)
    }
    pub fn should_continue_selection(
        &self,
        variable_values: &Option<HashMap<String, Value>>,
    ) -> bool {
        if let Some(skip_variable) = &self.skip_if_variables {
            for skip_variable in skip_variable {
                if let Some(variable_value) = variable_values
                    .as_ref()
                    .and_then(|vars| vars.get(skip_variable))
                {
                    if variable_value == &Value::Bool(true) {
                        return false; // Skip this field if the variable is true
                    }
                }
            }
        }
        if let Some(include_variable) = &self.include_if_variables {
            for include_variable in include_variable {
                if let Some(variable_value) = variable_values
                    .as_ref()
                    .and_then(|vars| vars.get(include_variable))
                {
                    if variable_value == &Value::Bool(true) {
                        return true; // Skip this field if the variable is not true
                    }
                }
            }
            return false;
        }
        true
    }
    pub fn satisfies_parent_type(&self, parent_type: &str) -> bool {
        if let Some(parent_conditions) = &self.parent_type_conditions {
            parent_conditions.contains(parent_type)
        } else {
            true
        }
    }
    pub fn satisfies_string_value(&self, value: &str) -> bool {
        if let Some(enum_values) = &self.enum_value_conditions {
            enum_values.contains(value)
        } else {
            true
        }
    }
    pub fn satisfies_object_value<'a>(&'a self, obj: &'a Map<String, Value>) -> Option<&'a str> {
        let type_name = obj
            .get(TYPENAME_FIELD)
            .and_then(|v| v.as_str())
            .unwrap_or(&self.type_name);
        if let Some(field_type_conditions) = &self.field_type_conditions {
            if field_type_conditions.contains(type_name) {
                Some(type_name)
            } else {
                warn!(
                    "Type {} is not in field type conditions: {:?}",
                    type_name, field_type_conditions
                );
                None
            }
        } else if type_name == self.type_name {
            return Some(type_name);
        } else {
            return None;
        }
    }
}

impl FieldProjectionPlan {
    pub fn from_selection_set(
        selection_set: &SelectionSet,
        schema_metadata: &SchemaMetadata,
        conditions: &FieldProjectionConditions,
    ) -> Option<Vec<FieldProjectionPlan>> {
        let mut field_selections: IndexMap<String, FieldProjectionPlan> = IndexMap::new();
        for selection_item in &selection_set.items {
            match selection_item {
                SelectionItem::Field(field) => {
                    let field_name = field.name.clone();
                    let response_key = field.alias.as_ref().unwrap_or(&field.name);
                    let field_type = if field_name == TYPENAME_FIELD {
                        "String"
                    } else {
                        let field_map = match schema_metadata.type_fields.get(&conditions.type_name)
                        {
                            Some(fields) => fields,
                            None => {
                                warn!(
                                    "No fields found for type {} in schema metadata.",
                                    conditions.type_name
                                );
                                return None;
                            }
                        };
                        match field_map.get(&field_name) {
                            Some(field_type) => field_type,
                            None => {
                                warn!(
                                    "Field {} not found in type {} in schema metadata.",
                                    field_name, conditions.type_name
                                );
                                continue;
                            }
                        }
                    };

                    if let Some(existing_field) = field_selections.get_mut(response_key.as_str()) {
                        if let Some(include_if) = &field.include_if {
                            // Skip the projection if it is not possible
                            if !existing_field
                                .conditions
                                .add_include_if_variable(include_if)
                            {
                                continue;
                            }
                        }
                        if let Some(skip_if) = &field.skip_if {
                            // Skip the projection if it is not possible
                            if !existing_field.conditions.add_skip_if_variable(skip_if) {
                                continue;
                            }
                        }
                        existing_field.conditions.extend(conditions);
                        if field.selections.items.is_empty() {
                            existing_field.selections = None;
                        } else if let Some(new_selections) = {
                            // Fork the conditions for the field type as parent for its subselections
                            if let Some(new_parent_conditions) =
                                existing_field.conditions.clone_for_parent_type(
                                    field_type,
                                    schema_metadata,
                                    field.include_if.as_ref(),
                                    field.skip_if.as_ref(),
                                )
                            {
                                FieldProjectionPlan::from_selection_set(
                                    &field.selections,
                                    schema_metadata,
                                    &new_parent_conditions,
                                )
                            } else {
                                continue;
                            }
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
                    } else if let Some(new_field_conditions) =
                        conditions.for_field_selection(field_type, field, schema_metadata)
                    {
                        let selections = if let Some(new_parent_conditions) = conditions
                            .clone_for_parent_type(
                                field_type,
                                schema_metadata,
                                field.include_if.as_ref(),
                                field.skip_if.as_ref(),
                            ) {
                            FieldProjectionPlan::from_selection_set(
                                &field.selections,
                                schema_metadata,
                                &new_parent_conditions,
                            )
                        } else {
                            None
                        };
                        let new_plan = FieldProjectionPlan {
                            field_name,
                            response_key: response_key.clone(),
                            conditions: new_field_conditions,
                            selections,
                        };
                        field_selections.insert(response_key.to_string(), new_plan);
                    } else {
                        continue;
                    }
                }
                SelectionItem::InlineFragment(inline_fragment) => {
                    if let Some(conditions) = conditions.clone_for_parent_type(
                        inline_fragment.type_condition.as_str(),
                        schema_metadata,
                        inline_fragment.include_if.as_ref(),
                        inline_fragment.skip_if.as_ref(),
                    ) {
                        if let Some(inline_fragment_selections) =
                            FieldProjectionPlan::from_selection_set(
                                &inline_fragment.selections,
                                schema_metadata,
                                &conditions,
                            )
                        {
                            for selection in inline_fragment_selections {
                                if let Some(existing_field) =
                                    field_selections.get_mut(selection.response_key.as_str())
                                {
                                    existing_field.conditions.extend(&selection.conditions);
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
    ) -> (&'static str, Vec<FieldProjectionPlan>) {
        let root_type_name = match operation.operation_kind {
            Some(OperationKind::Query) => "Query",
            Some(OperationKind::Mutation) => "Mutation",
            Some(OperationKind::Subscription) => "Subscription",
            None => "Query",
        };

        let field_type_conditions = schema_metadata
            .possible_types
            .get_possible_types(root_type_name);
        let conditions = FieldProjectionConditions {
            type_name: root_type_name.to_owned(),
            skip_if_variables: None,
            include_if_variables: None,
            parent_type_conditions: None,
            field_type_conditions: Some(field_type_conditions),
            enum_value_conditions: None,
        };
        (
            root_type_name,
            FieldProjectionPlan::from_selection_set(
                &operation.selection_set,
                schema_metadata,
                &conditions,
            )
            .unwrap_or_default(),
        )
    }
}

#[instrument(level = "trace", skip_all)]
pub fn project_by_operation(
    data: &mut Value,
    errors: &mut Vec<GraphQLError>,
    extensions: &HashMap<String, Value>,
    operation_type_name: &str,
    selections: &Vec<FieldProjectionPlan>,
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
    selection: &FieldProjectionPlan,
    variable_values: &Option<HashMap<String, Value>>,
    buffer: &mut String,
) {
    match data {
        Value::Null => buffer.push_str("null"),
        Value::Bool(true) => buffer.push_str("true"),
        Value::Bool(false) => buffer.push_str("false"),
        Value::Number(num) => write!(buffer, "{}", num).unwrap(),
        Value::String(value) => {
            if !selection.conditions.satisfies_string_value(value) {
                errors.push(GraphQLError {
                    message: "Value is not a valid enum value".to_string(),
                    locations: None,
                    path: None,
                    extensions: None,
                });
                buffer.push_str("null");
                return;
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
                project_selection_set(item, errors, selection, variable_values, buffer);
                first = false;
            }
            buffer.push(']');
        }
        Value::Object(obj) => {
            match selection.selections.as_ref() {
                Some(selections) => {
                    if let Some(type_name) = selection.conditions.satisfies_object_value(obj) {
                        let mut first = true;
                        project_selection_set_with_map(
                            obj,
                            errors,
                            selections,
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
                    } else {
                        buffer.push_str("null");
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
    selections: &Vec<FieldProjectionPlan>,
    variable_values: &Option<HashMap<String, Value>>,
    parent_type_name: &str,
    buffer: &mut String,
    first: &mut bool,
) {
    for selection in selections {
        if !selection
            .conditions
            .should_continue_selection(variable_values)
        {
            // If the selection is not included by variable, skip it
            continue;
        }
        if !selection.conditions.satisfies_parent_type(parent_type_name) {
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

        let field_val = obj.get(selection.response_key.as_str());

        if let Some(field_val) = field_val {
            project_selection_set(field_val, errors, selection, variable_values, buffer);
        } else {
            // If the field is not found in the object, set it to Null
            buffer.push_str("null");
            continue;
        }
    }
}
