use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::primitives::expression::Expression;

/// A map of label names to their override configuration.
pub type OverrideLabelsConfig = HashMap<String, LabelOverrideValue>;

/// Defines the value for a label override.
///
/// It can be a simple boolean,
/// or an object containing the expression that evaluates to a boolean.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum LabelOverrideValue {
    /// A static boolean value to enable or disable the label.
    Boolean(bool),
    /// A dynamic value computed by an expression.
    Expression(Expression),
}

impl LabelOverrideValue {
    pub fn is_bool_and_true(&self) -> bool {
        matches!(self, LabelOverrideValue::Boolean(true))
    }
}
