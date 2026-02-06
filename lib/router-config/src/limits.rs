use human_size::Size;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
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

    /// Configuration of limiting the number of aliases in the incoming GraphQL operations.
    /// If not specified, alias limiting is disabled.
    ///
    /// It is used to prevent too many aliases that could lead to overfetching or DOS attacks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_aliases: Option<MaxAliasesRuleConfig>,

    #[serde(default = "default_max_request_body_size")]
    #[schemars(with = "String")]
    pub max_request_body_size: Size,
}

impl Default for LimitsConfig {
    fn default() -> Self {
        Self {
            max_depth: None,
            max_directives: None,
            max_tokens: None,
            max_aliases: None,
            max_request_body_size: default_max_request_body_size(),
        }
    }
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
}

fn default_ignore_introspection() -> bool {
    true
}

fn default_flatten_fragments() -> bool {
    false
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct MaxDirectivesRuleConfig {
    /// Directives threshold
    pub n: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct MaxTokensRuleConfig {
    /// Tokens threshold
    pub n: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct MaxAliasesRuleConfig {
    /// Aliases threshold
    pub n: usize,
}

fn default_max_request_body_size() -> Size {
    "2MB".parse().expect(
        "Default value for 'limits.max_request_body_size' should be a valid human-readable size",
    )
}
