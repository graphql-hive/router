use std::{collections::HashMap, time::Duration};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct TrafficShapingConfig {
    /// The default configuration that will be applied to all subgraphs, unless overridden by a specific subgraph configuration.
    #[serde(default)]
    pub all: TrafficShapingExecutorConfig,
    /// Optional per-subgraph configurations that will override the default configuration for specific subgraphs.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub subgraphs: HashMap<String, TrafficShapingExecutorSubgraphConfig>,
    /// Limits the concurrent amount of requests/connections per host/subgraph.
    #[serde(default = "default_max_connections_per_host")]
    pub max_connections_per_host: usize,
}

impl Default for TrafficShapingConfig {
    fn default() -> Self {
        Self {
            all: TrafficShapingExecutorConfig::default(),
            subgraphs: HashMap::new(),
            max_connections_per_host: default_max_connections_per_host(),
        }
    }
}

fn default_max_connections_per_host() -> usize {
    100
}

fn default_pool_idle_timeout_seconds() -> u64 {
    50
}

fn default_dedupe_enabled() -> bool {
    true
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct TrafficShapingExecutorSubgraphConfig {
    /// Timeout for idle sockets being kept-alive.
    pub pool_idle_timeout_seconds: Option<u64>,

    /// Enables/disables request deduplication to subgraphs.
    ///
    /// When requests exactly matches the hashing mechanism (e.g., subgraph name, URL, headers, query, variables), and are executed at the same time, they will
    /// be deduplicated by sharing the response of other in-flight requests.
    pub dedupe_enabled: Option<bool>,

    /// Optional timeout configuration for requests to subgraphs.
    ///
    /// Example with a fixed duration:
    /// ```yaml
    ///   timeout:
    ///     duration: 5s
    /// ```
    ///
    /// Or with a VRL expression that can return a duration based on the operation kind:
    /// ```yaml
    ///   timeout:
    ///     expression: |
    ///      if (.request.operation.type == "mutation") {
    ///        "10s"
    ///      } else {
    ///        "15s"
    ///      }
    /// ```
    pub request_timeout: Option<DurationOrExpression>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct TrafficShapingExecutorConfig {
    /// Timeout for idle sockets being kept-alive.
    #[serde(default = "default_pool_idle_timeout_seconds")]
    pub pool_idle_timeout_seconds: u64,

    /// Enables/disables request deduplication to subgraphs.
    ///
    /// When requests exactly matches the hashing mechanism (e.g., subgraph name, URL, headers, query, variables), and are executed at the same time, they will
    /// be deduplicated by sharing the response of other in-flight requests.
    #[serde(default = "default_dedupe_enabled")]
    pub dedupe_enabled: bool,

    /// Optional timeout configuration for requests to subgraphs.
    ///
    /// Example with a fixed duration:
    /// ```yaml
    ///   timeout:
    ///     duration: 5s
    /// ```
    ///
    /// Or with a VRL expression that can return a duration based on the operation kind:
    /// ```yaml
    ///   timeout:
    ///     expression: |
    ///      if (.request.operation.type == "mutation") {
    ///        "10s"
    ///      } else {
    ///        "15s"
    ///      }
    /// ```
    #[serde(default = "default_request_timeout")]
    pub request_timeout: DurationOrExpression,
}

fn default_request_timeout() -> DurationOrExpression {
    DurationOrExpression::Duration(Duration::from_secs(15))
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
pub enum DurationOrExpression {
    /// A fixed duration, e.g., "5s" or "100ms".
    #[serde(
        deserialize_with = "humantime_serde::deserialize",
        serialize_with = "humantime_serde::serialize"
    )]
    Duration(Duration),
    /// A VRL expression that evaluates to a duration. The result can be an integer (milliseconds), a float (milliseconds), or a duration string (e.g. "5s").
    Expression { expression: String },
}

impl Default for TrafficShapingExecutorConfig {
    fn default() -> Self {
        Self {
            pool_idle_timeout_seconds: default_pool_idle_timeout_seconds(),
            dedupe_enabled: default_dedupe_enabled(),
            request_timeout: default_request_timeout(),
        }
    }
}
