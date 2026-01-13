use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, Default)]
pub struct LimitsConfig {
    /// Configuration of limiting the depth of the incoming GraphQL operations.
    /// If not specified, depth limiting is disabled.
    ///
    /// It is used to prevent too large queries that could lead to overfetching or DOS attacks.
    #[serde(default)]
    pub max_depth: Option<MaxDepthRuleConfig>,

    /// Configuration of limiting the number of directives in the incoming GraphQL operations.
    /// If not specified, directive limiting is disabled.
    ///
    /// It is used to prevent too many directives that could lead to overfetching or DOS attacks.
    #[serde(default)]
    pub max_directives: Option<MaxDirectivesRuleConfig>,

    /// Configuration of limiting the number of tokens in the incoming GraphQL operations.
    /// If not specified, token limiting is disabled.
    ///
    /// It is used to prevent too large queries that could lead to overfetching or DOS attacks.
    #[serde(default)]
    pub max_tokens: Option<MaxTokensRuleConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct MaxDepthRuleConfig {
    #[serde(default = "default_max_depth")]
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

impl Default for MaxDepthRuleConfig {
    fn default() -> Self {
        MaxDepthRuleConfig {
            n: default_max_depth(),
            ignore_introspection: default_ignore_introspection(),
            flatten_fragments: default_flatten_fragments(),
            expose_limits: default_expose_limits(),
        }
    }
}

fn default_max_depth() -> usize {
    6
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
    #[serde(default = "default_max_directives")]
    /// Directives threshold
    pub n: usize,

    #[serde(default = "default_expose_limits")]
    /// Whether to expose the limits in the error message.
    pub expose_limits: bool,
}

fn default_max_directives() -> usize {
    50
}

impl Default for MaxDirectivesRuleConfig {
    fn default() -> Self {
        MaxDirectivesRuleConfig {
            n: default_max_directives(),
            expose_limits: default_expose_limits(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct MaxTokensRuleConfig {
    #[serde(default = "default_max_tokens")]
    /// Tokens threshold
    pub n: usize,

    #[serde(default = "default_expose_limits")]
    /// Whether to expose the limits in the error message.
    pub expose_limits: bool,
}

fn default_max_tokens() -> usize {
    1000
}

impl Default for MaxTokensRuleConfig {
    fn default() -> Self {
        MaxTokensRuleConfig {
            n: default_max_tokens(),
            expose_limits: default_expose_limits(),
        }
    }
}
