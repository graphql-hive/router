use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use std::collections::HashMap;

#[derive(Clone, Deserialize, Serialize, JsonSchema)]
pub struct TrafficShapingConfig {
    /// The default configuration that will be applied to all subgraphs, unless overridden by a specific subgraph configuration.
    #[serde(default)]
    pub all: TrafficShapingExecutorConfig,
    /// Optional per-subgraph configurations that will override the default configuration for specific subgraphs.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub subgraphs: HashMap<String, TrafficShapingExecutorConfig>,
    /// Limits the concurrent amount of requests/connections per host/subgraph.
    #[serde(default = "default_max_connections_per_host")]
    pub max_connections_per_host: usize,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
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
    ///        10000
    ///      } else {
    ///        5000
    ///      }
    /// ```
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<SubgraphTimeoutConfig>,

    #[serde(default = "default_max_retries")]
    pub max_retries: usize,
    
    #[serde(deserialize_with = "humantime_serde", default = "default_retry_delay")]
    pub retry_delay: Duration,

    #[serde(default = "default_retry_factor")]
    pub retry_factor: u64,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(rename_all = "camelCase")]
pub enum SubgraphTimeoutConfig {
    Expression(String),
    #[serde(deserialize_with = "humantime_serde")]
    Duration(Duration),
}

fn humantime_serde<'de, D>(deserializer: D) -> Result<Duration, D::Error>
where
    D: serde::Deserializer<'de>,
{
    humantime_serde::deserialize(deserializer)
}

impl Default for TrafficShapingExecutorConfig {
    fn default() -> Self {
        Self {
            pool_idle_timeout_seconds: default_pool_idle_timeout_seconds(),
            dedupe_enabled: default_dedupe_enabled(),
            timeout: None,
            max_retries: 0,
            retry_delay: default_retry_delay(),
            retry_factor: default_retry_factor(),
        }
    }
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

fn default_max_retries() -> usize {
    0
}

fn default_retry_delay() -> Duration {
    Duration::from_secs(1)
}

fn default_retry_factor() -> u64 {
    1
}