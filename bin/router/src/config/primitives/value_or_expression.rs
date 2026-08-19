use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum ValueOrExpression<T: Default> {
    Value(T),
    Expression { expression: String },
}

impl<T: Default> Default for ValueOrExpression<T> {
    fn default() -> Self {
        ValueOrExpression::Value(T::default())
    }
}
