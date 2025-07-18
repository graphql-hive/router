use std::{collections::HashMap, io};

use query_planner::{
    ast::{
        operation::OperationDefinition, selection_item::SelectionItem, selection_set::SelectionSet,
    },
    state::supergraph_state::OperationKind,
};
use serde_json::{Map, Value};
use tracing::{instrument, warn};

use crate::json_writer::write_and_escape_string_writer;
use crate::{schema_metadata::SchemaMetadata, GraphQLError, TYPENAME_FIELD};

#[instrument(level = "trace", skip_all)]
pub fn project_by_operation(
    data: &Value,
    errors: &mut Vec<GraphQLError>,
    extensions: &HashMap<String, Value>,
    operation: &OperationDefinition,
    schema_metadata: &SchemaMetadata,
    variable_values: &Option<HashMap<String, Value>>,
    buffer: &mut impl io::Write,
) -> io::Result<()> {
    let root_type_name = match operation.operation_kind {
        Some(OperationKind::Query) => "Query",
        Some(OperationKind::Mutation) => "Mutation",
        Some(OperationKind::Subscription) => "Subscription",
        None => "Query",
    };

    buffer.write_all(b"{\"data\":")?;

    project_selection_set(
        data,
        errors,
        &operation.selection_set,
        root_type_name,
        schema_metadata,
        variable_values,
        buffer,
    )?;

    // if !errors.is_empty() {
    //     buffer.write_all(b",\"errors\":")?;
    //     serde_json::to_writer(buffer, &errors).unwrap()
    //     // buffer.write(serde_json::to(&errors).unwrap())
    //     // write!(
    //     //     buffer,
    //     //     "{}",
    //     //     serde_json::to_string(&errors).unwrap()
    //     // )?
    // }
    // if !extensions.is_empty() {
    //     write!(
    //         buffer,
    //         ",\"extensions\":{}",
    //         serde_json::to_string(&extensions).unwrap()
    //     )?;
    // }

    buffer.write_all(b"}")
}

#[allow(clippy::too_many_arguments)]
fn project_array(
    arr: &[Value],
    errors: &mut Vec<GraphQLError>,
    selection_set: &SelectionSet,
    type_name: &str,
    schema_metadata: &SchemaMetadata,
    variable_values: &Option<HashMap<String, Value>>,
    buffer: &mut impl io::Write,
) -> Result<(), io::Error> {
    buffer.write_all(b"[")?;
    let mut first = true;
    for item in arr.iter() {
        if !first {
            buffer.write_all(b",")?;
        }
        project_selection_set(
            item,
            errors,
            selection_set,
            type_name,
            schema_metadata,
            variable_values,
            buffer,
        )?;
        first = false;
    }
    buffer.write_all(b"]")
}

#[allow(clippy::too_many_arguments)]
fn project_object(
    obj: &Map<String, Value>,
    errors: &mut Vec<GraphQLError>,
    selection_set: &SelectionSet,
    type_name: &str,
    schema_metadata: &SchemaMetadata,
    variable_values: &Option<HashMap<String, Value>>,
    buffer: &mut impl io::Write,
) -> Result<(), io::Error> {
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
    )?;
    if !is_projected {
        buffer.write_all(b"null")
    } else if !first {
        buffer.write_all(b"}")
    } else {
        buffer.write_all(b"{}")
    }
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
    buffer: &mut impl io::Write,
) -> Result<(), io::Error> {
    match data {
        Value::Null => buffer.write_all(b"null"),
        Value::Bool(true) => buffer.write_all(b"true"),
        Value::Bool(false) => buffer.write_all(b"false"),
        Value::Number(num) => buffer.write_all(num.to_string().as_bytes()),
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
                    return buffer.write_all(b"null");
                }
            }
            write_and_escape_string_writer(buffer, value)?;
            Ok(())
        }
        Value::Array(arr) => project_array(
            arr,
            errors,
            selection_set,
            type_name,
            schema_metadata,
            variable_values,
            buffer,
        ),
        Value::Object(obj) => project_object(
            obj,
            errors,
            selection_set,
            type_name,
            schema_metadata,
            variable_values,
            buffer,
        ),
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
    buffer: &mut impl io::Write,
    first: &mut bool,
) -> Result<bool, io::Error> {
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
            return Ok(false);
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
                    buffer.write_all(b"{")?;
                } else {
                    buffer.write_all(b",")?;
                }
                *first = false;

                if field.name == TYPENAME_FIELD {
                    // write!(buffer, "\"{}\": \"{}\"", response_key, type_name)?;
                    buffer.write_all(b"\"")?;
                    buffer.write(response_key.as_bytes())?;
                    buffer.write_all(b"\": \"")?;
                    buffer.write(type_name.as_bytes())?;
                    buffer.write_all(b"\"")?;
                    continue;
                }

                buffer.write_all(b"\"")?;
                buffer.write(response_key.as_bytes())?;
                buffer.write_all(b"\":")?;

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
                    )?;
                } else {
                    // If the field is not found in the object, set it to Null
                    buffer.write_all(b"null")?;
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
                    )?;
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
    Ok(true)
}
