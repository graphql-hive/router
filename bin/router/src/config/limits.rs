use human_size::Size;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[non_exhaustive]
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

    /// Limits how many `@defer`s one operation can have, `@defer(if:)` included.
    /// Only applies with `defer.enabled` on.
    ///
    /// Every `@defer` is a deferred part with fetches of its own, and a `@defer(if:)` is planned
    /// both ways, deferred and not, so each one makes the query plan bigger. Defaults to 4.
    /// Set it to `null` to turn it off.
    #[serde(default = "default_max_defers")]
    pub max_defers: Option<MaxDefersRuleConfig>,

    #[serde(default = "default_max_request_body_size")]
    #[schemars(with = "String")]
    pub max_request_body_size: Size,

    /// The maximum total size of the incoming HTTP request headers.
    /// Requests exceeding this limit are rejected with `431 Request Header Fields Too Large`
    /// before being processed, so oversized headers (e.g. large cookies) never reach the subgraphs.
    ///
    /// Defaults to `64KiB`.
    #[serde(default = "default_max_request_header_size")]
    #[schemars(with = "String")]
    pub max_request_header_size: Size,
}

impl Default for LimitsConfig {
    fn default() -> Self {
        Self {
            max_depth: None,
            max_directives: None,
            max_tokens: None,
            max_aliases: None,
            max_defers: default_max_defers(),
            max_request_body_size: default_max_request_body_size(),
            max_request_header_size: default_max_request_header_size(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[non_exhaustive]
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
#[non_exhaustive]
pub struct MaxDirectivesRuleConfig {
    /// Directives threshold
    pub n: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[non_exhaustive]
pub struct MaxTokensRuleConfig {
    /// Tokens threshold
    pub n: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[non_exhaustive]
pub struct MaxAliasesRuleConfig {
    /// Aliases threshold
    pub n: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[non_exhaustive]
pub struct MaxDefersRuleConfig {
    /// How many `@defer`s an operation can have
    pub n: usize,
}

fn default_max_defers() -> Option<MaxDefersRuleConfig> {
    Some(MaxDefersRuleConfig { n: 4 })
}

fn default_max_request_body_size() -> Size {
    "2MB".parse().expect(
        "Default value for 'limits.max_request_body_size' should be a valid human-readable size",
    )
}

fn default_max_request_header_size() -> Size {
    // Matches ntex's default HTTP message buffer size (64 * 1024 bytes).
    "64KiB".parse().expect(
        "Default value for 'limits.max_request_header_size' should be a valid human-readable size",
    )
}
