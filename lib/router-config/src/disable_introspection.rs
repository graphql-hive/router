use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Defines the value for disabling introspection queries.
///
/// It can be a simple boolean,
/// or an object containing the expression that evaluates to a boolean.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum DisableIntrospectionConfig {
    /// A static boolean value to enable or disable the label.
    Boolean(bool),
    /// A dynamic value computed by an expression.
    Expression {
        /// An expression that must evaluate to a boolean. If true, the label will be applied.
        expression: String,
    },
}
