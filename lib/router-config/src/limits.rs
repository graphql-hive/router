use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, Default)]
pub struct LimitsConfig {
    /// Configuration of limiting the depth of the incoming GraphQL operations.
    /// If not specified, depth limiting is disabled.
    ///
    /// It is used to prevent too large queries that could lead to overfetching or DOS attacks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_depth: Option<MaxDepthRuleConfig>,

    /// Configuration of limiting the number of directives in the incoming GraphQL operations.
    /// If not specified, directive limiting is disabled.
    ///
    /// It is used to prevent too many directives that could lead to overfetching or DOS attacks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_directives: Option<MaxDirectivesRuleConfig>,

    /// Configuration of limiting the number of tokens in the incoming GraphQL operations.
    /// If not specified, token limiting is disabled.
    ///
    /// It is used to prevent too large queries that could lead to overfetching or DOS attacks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<MaxTokensRuleConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct MaxDepthRuleConfig {
    /// Depth threshold
    pub n: usize,

    #[serde(default = "default_ignore_introspection")]
    /// Ignore the depth of introspection queries.
    pub ignore_introspection: bool,

    #[serde(default = "default_flatten_fragments")]
    /// Flatten fragment spreads and inline fragments when calculating depth.
    pub flatten_fragments: bool,

    #[serde(default = "default_expose_limits")]
    /// Whether to expose the limits in the error message.
    pub expose_limits: bool,
}

fn default_ignore_introspection() -> bool {
    true
}

fn default_flatten_fragments() -> bool {
    false
}

fn default_expose_limits() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct MaxDirectivesRuleConfig {
    /// Directives threshold
    pub n: usize,

    #[serde(default = "default_expose_limits")]
    /// Whether to expose the limits in the error message.
    pub expose_limits: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct MaxTokensRuleConfig {
    /// Tokens threshold
    pub n: usize,

    #[serde(default = "default_expose_limits")]
    /// Whether to expose the limits in the error message.
    pub expose_limits: bool,
}
