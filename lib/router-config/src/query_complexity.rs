use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, Default)]
pub struct QueryComplexityConfig {
    pub max_depth: Option<MaxDepthRuleConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct MaxDepthRuleConfig {
    #[serde(default = "default_max_depth")]
    /// Depth threshold. A value of 0 means no limit.
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

    #[serde(default = "default_propagate_on_rejection")]
    /// Whether to propagate the error when the depth limit is exceeded.
    pub propagate_on_rejection: bool,
}

impl MaxDepthRuleConfig {
    pub fn is_enabled(&self) -> bool {
        self.n > 0
    }
}

impl Default for MaxDepthRuleConfig {
    fn default() -> Self {
        MaxDepthRuleConfig {
            n: default_max_depth(),
            ignore_introspection: default_ignore_introspection(),
            flatten_fragments: default_flatten_fragments(),
            expose_limits: default_expose_limits(),
            propagate_on_rejection: default_propagate_on_rejection(),
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

fn default_propagate_on_rejection() -> bool {
    true
}
