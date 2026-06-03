use hive_console_sdk::expressions::{CompileExpression, ExecutableProgram};
use hive_router_config::primitives::value_or_expression::ValueOrExpression;
use vrl::core::Value;

use crate::storage::error::StorageError;

pub trait FromVrlValue: Sized {
    fn from_vrl_value(value: &Value, context: &str) -> Result<Self, StorageError>;
}

impl FromVrlValue for String {
    fn from_vrl_value(value: &Value, context: &str) -> Result<Self, StorageError> {
        value
            .as_str()
            .ok_or_else(|| {
                StorageError::Configuration(format!("{} expression must return a string", context))
            })
            .map(|s| s.to_string())
    }
}

impl FromVrlValue for bool {
    fn from_vrl_value(value: &Value, context: &str) -> Result<Self, StorageError> {
        value.as_boolean().ok_or_else(|| {
            StorageError::Configuration(format!("{} expression must return a boolean", context))
        })
    }
}

fn evaluate_expression<T: FromVrlValue>(
    expression: &str,
    context: &str,
) -> Result<T, StorageError> {
    let value = expression
        .compile_expression(None)
        .map_err(|e| {
            StorageError::Configuration(format!("Failed to compile {} expression: {}", context, e))
        })?
        .execute(Value::Null)
        .map_err(|e| {
            StorageError::Configuration(format!("Failed to execute {} expression: {}", context, e))
        })?;

    T::from_vrl_value(&value, context)
}

pub fn resolve_value_or_expression<T>(
    value_or_expr: &ValueOrExpression<T>,
    context: &str,
) -> Result<T, StorageError>
where
    T: FromVrlValue + Clone + Default,
{
    match value_or_expr {
        ValueOrExpression::Value(v) => Ok(v.clone()),
        ValueOrExpression::Expression { expression } => {
            evaluate_expression::<T>(expression, context)
        }
    }
}
