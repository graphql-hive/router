use std::collections::HashMap;
use std::fmt;

use crate::query_planner::state::supergraph_state::TypeNode;
use sonic_rs::{JsonNumberTrait, Value, ValueRef};

use crate::executor::introspection::schema::SchemaMetadata;

/// A request error raised while coercing an operation's variable values
/// (spec: "Coercing Variable Values"). Messages follow graphql-js.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum VariableCoercionError {
    #[error("Variable \"${name}\" has invalid value{path}: {reason}")]
    InvalidValue {
        name: String,
        path: ValuePath,
        reason: InvalidValueReason,
    },

    #[error("Variable \"${name}\" has invalid default value{path}: {reason}")]
    InvalidDefaultValue {
        name: String,
        path: ValuePath,
        reason: InvalidValueReason,
    },
}

/// Why a value cannot be coerced to its expected type.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum InvalidValueReason {
    #[error("Expected a value of non-null type \"{type_name}\" to be provided.")]
    MissingNonNullValue { type_name: String },

    #[error("Expected value of non-null type \"{type_name}\" not to be null.")]
    UnexpectedNull { type_name: String },

    #[error("Value \"{value}\" does not exist in \"{type_name}\" enum.")]
    InvalidEnumValue { value: String, type_name: String },

    #[error("Enum \"{type_name}\" cannot represent non-string value: {value}.")]
    ExpectedEnumString { type_name: String, value: String },

    #[error("String cannot represent a non string value: {value}")]
    ExpectedString { value: String },

    #[error("ID cannot represent value: {value}")]
    ExpectedId { value: String },

    #[error("Int cannot represent non-integer value: {value}")]
    ExpectedInteger { value: String },

    #[error("Float cannot represent non numeric value: {value}")]
    ExpectedFloat { value: String },

    #[error("Boolean cannot represent a non boolean value: {value}")]
    ExpectedBoolean { value: String },
}

/// Where the invalid value sits inside a variable's value, as list indices from the outside in.
/// Displayed like graphql-js: empty for the variable itself, otherwise ` at [0][1]`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ValuePath(Vec<usize>);

impl fmt::Display for ValuePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.is_empty() {
            return Ok(());
        }
        f.write_str(" at ")?;
        for index in &self.0 {
            write!(f, "[{index}]")?;
        }
        Ok(())
    }
}

/// An invalid value found by `validate_runtime_value`. The path is collected while the error
/// travels up, so it holds the innermost index first.
#[derive(Debug, PartialEq)]
struct InvalidValue {
    reversed_path: Vec<usize>,
    reason: InvalidValueReason,
}

impl InvalidValue {
    fn new(reason: InvalidValueReason) -> Self {
        InvalidValue {
            reversed_path: Vec::new(),
            reason,
        }
    }

    fn path(mut self) -> (ValuePath, InvalidValueReason) {
        self.reversed_path.reverse();
        (ValuePath(self.reversed_path), self.reason)
    }
}

#[inline]
pub fn collect_variables(
    operation: &crate::query_planner::ast::operation::OperationDefinition,
    variables_map: &mut HashMap<String, Value>,
    schema_metadata: &SchemaMetadata,
) -> Result<Option<HashMap<String, Value>>, VariableCoercionError> {
    let Some(variable_definitions) = operation.variable_definitions.as_ref() else {
        return Ok(None);
    };

    let mut variable_values: HashMap<String, Value> =
        HashMap::with_capacity(variable_definitions.len());

    for variable_definition in variable_definitions {
        let variable_name = variable_definition.name.as_str();
        let variable_type = &variable_definition.variable_type;

        if let Some(variable_value) = variables_map.remove(variable_name) {
            validate_runtime_value(variable_value.as_ref(), variable_type, schema_metadata)
                .map_err(|invalid| {
                    let (path, reason) = invalid.path();
                    VariableCoercionError::InvalidValue {
                        name: variable_name.to_string(),
                        path,
                        reason,
                    }
                })?;
            variable_values.insert(variable_name.to_string(), variable_value);
            continue;
        }

        if let Some(default_value) = &variable_definition.default_value {
            let default_value_coerced: Value = default_value.into();
            validate_runtime_value(
                default_value_coerced.as_ref(),
                variable_type,
                schema_metadata,
            )
            .map_err(|invalid| {
                let (path, reason) = invalid.path();
                VariableCoercionError::InvalidDefaultValue {
                    name: variable_name.to_string(),
                    path,
                    reason,
                }
            })?;
            variable_values.insert(variable_name.to_string(), default_value_coerced);
            continue;
        }

        if variable_type.is_non_null() {
            return Err(VariableCoercionError::InvalidValue {
                name: variable_name.to_string(),
                path: ValuePath::default(),
                reason: InvalidValueReason::MissingNonNullValue {
                    type_name: variable_type.to_string(),
                },
            });
        }
    }

    if variable_values.is_empty() {
        Ok(None)
    } else {
        Ok(Some(variable_values))
    }
}

#[inline]
fn validate_runtime_value(
    value: ValueRef,
    type_node: &TypeNode,
    schema_metadata: &SchemaMetadata,
) -> Result<(), InvalidValue> {
    if let ValueRef::Null = value {
        return if type_node.is_non_null() {
            Err(InvalidValue::new(InvalidValueReason::UnexpectedNull {
                type_name: type_node.to_string(),
            }))
        } else {
            Ok(())
        };
    }
    match type_node {
        TypeNode::Named(name) => {
            if let Some(enum_values) = schema_metadata.enum_values.get(name) {
                if let ValueRef::String(s) = value {
                    if !enum_values.contains(s) {
                        return Err(InvalidValue::new(InvalidValueReason::InvalidEnumValue {
                            value: s.to_string(),
                            type_name: name.clone(),
                        }));
                    }
                } else {
                    return Err(InvalidValue::new(InvalidValueReason::ExpectedEnumString {
                        type_name: name.clone(),
                        value: inspect(value),
                    }));
                }
            } else {
                let is_valid = match name.as_str() {
                    "String" => matches!(value, ValueRef::String(_)),
                    "ID" => matches!(value, ValueRef::String(_)),
                    "Int" => matches!(value, ValueRef::Number(ref num) if num.is_i64()),
                    "Float" => matches!(
                        value,
                        ValueRef::Number(ref num) if num.is_f64() || num.is_i64()
                    ),
                    "Boolean" => matches!(value, ValueRef::Bool(_)),
                    // Custom scalars and input objects
                    _ => true,
                };
                if !is_valid {
                    let value = inspect(value);
                    return Err(InvalidValue::new(match name.as_str() {
                        "String" => InvalidValueReason::ExpectedString { value },
                        "ID" => InvalidValueReason::ExpectedId { value },
                        "Int" => InvalidValueReason::ExpectedInteger { value },
                        "Float" => InvalidValueReason::ExpectedFloat { value },
                        _ => InvalidValueReason::ExpectedBoolean { value },
                    }));
                }
            }
        }
        TypeNode::NonNull(inner_type) => {
            // The null check is now handled above, so we can just recurse.
            validate_runtime_value(value, inner_type, schema_metadata)?;
        }
        TypeNode::List(inner_type) => {
            if let ValueRef::Array(arr) = value {
                for (index, item) in arr.iter().enumerate() {
                    validate_runtime_value(item.as_ref(), inner_type, schema_metadata).map_err(
                        |mut invalid| {
                            invalid.reversed_path.push(index);
                            invalid
                        },
                    )?;
                }
            } else {
                validate_runtime_value(value, inner_type, schema_metadata)?;
            }
        }
    }
    Ok(())
}

/// How graphql-js prints a value in error messages (its `inspect` utility).
fn inspect(value: ValueRef) -> String {
    let mut output = String::new();
    write_inspected(&mut output, value, 0);
    output
}

/// graphql-js prints at most this many list items, and replaces lists and objects nested
/// deeper than `MAX_INSPECT_DEPTH` with `[Array]` / `[Object]`.
const MAX_INSPECT_LIST_ITEMS: usize = 10;
const MAX_INSPECT_DEPTH: usize = 2;

fn write_inspected(output: &mut String, value: ValueRef, depth: usize) {
    match value {
        ValueRef::Null => output.push_str("null"),
        ValueRef::Bool(b) => output.push_str(if b { "true" } else { "false" }),
        ValueRef::Number(num) => match (num.as_i64(), num.as_u64(), num.as_f64()) {
            (Some(n), _, _) => output.push_str(&n.to_string()),
            (_, Some(n), _) => output.push_str(&n.to_string()),
            (_, _, Some(n)) => output.push_str(&n.to_string()),
            _ => output.push_str(&num.to_string()),
        },
        ValueRef::String(s) => {
            output.push_str(&sonic_rs::to_string(s).unwrap_or_else(|_| format!("{s:?}")))
        }
        ValueRef::Array(arr) => {
            if arr.is_empty() {
                output.push_str("[]");
            } else if depth >= MAX_INSPECT_DEPTH {
                output.push_str("[Array]");
            } else {
                output.push('[');
                for (index, item) in arr.iter().take(MAX_INSPECT_LIST_ITEMS).enumerate() {
                    if index > 0 {
                        output.push_str(", ");
                    }
                    write_inspected(output, item.as_ref(), depth + 1);
                }
                match arr.len().saturating_sub(MAX_INSPECT_LIST_ITEMS) {
                    0 => {}
                    1 => output.push_str(", ... 1 more item"),
                    remaining => output.push_str(&format!(", ... {remaining} more items")),
                }
                output.push(']');
            }
        }
        ValueRef::Object(obj) => {
            if obj.is_empty() {
                output.push_str("{}");
            } else if depth >= MAX_INSPECT_DEPTH {
                output.push_str("[Object]");
            } else {
                output.push_str("{ ");
                for (index, (key, item)) in obj.iter().enumerate() {
                    if index > 0 {
                        output.push_str(", ");
                    }
                    output.push_str(key);
                    output.push_str(": ");
                    write_inspected(output, item.as_ref(), depth + 1);
                }
                output.push_str(" }");
            }
        }
    }
}

#[cfg(test)]
mod tests;
