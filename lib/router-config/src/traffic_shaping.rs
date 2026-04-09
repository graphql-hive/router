use std::{collections::HashMap, time::Duration};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::primitives::{http_header::HttpHeaderName, percentage::Percentage};

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct TrafficShapingConfig {
    /// The default configuration that will be applied to all subgraphs, unless overridden by a specific subgraph configuration.
    #[serde(default)]
    pub all: TrafficShapingExecutorGlobalConfig,
    /// Optional per-subgraph configurations that will override the default configuration for specific subgraphs.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub subgraphs: HashMap<String, TrafficShapingExecutorSubgraphConfig>,
    /// Limits the concurrent amount of requests/connections per host/subgraph.
    #[serde(default = "default_max_connections_per_host")]
    pub max_connections_per_host: usize,

    #[serde(default)]
    /// Configuration for the router itself, e.g., for handling incoming requests, or other router-level traffic shaping configurations.
    pub router: TrafficShapingRouterConfig,
}

impl Default for TrafficShapingConfig {
    fn default() -> Self {
        Self {
            all: TrafficShapingExecutorGlobalConfig::default(),
            subgraphs: HashMap::new(),
            max_connections_per_host: default_max_connections_per_host(),
            router: TrafficShapingRouterConfig::default(),
        }
    }
}

fn default_max_connections_per_host() -> usize {
    100
}

fn default_pool_idle_timeout() -> Duration {
    Duration::from_secs(50)
}

fn default_dedupe_enabled() -> bool {
    true
}

fn default_router_dedupe_enabled() -> bool {
    false
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct TrafficShapingExecutorSubgraphConfig {
    /// Timeout for idle sockets being kept-alive.
    #[serde(
        deserialize_with = "humantime_serde::deserialize",
        serialize_with = "humantime_serde::serialize",
        skip_serializing_if = "Option::is_none",
        default = "default_subgraph_pool_idle_timeout"
    )]
    #[schemars(with = "Option<String>")]
    pub pool_idle_timeout: Option<Duration>,

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

    /// Circuit Breaker configuration for the subgraph.
    /// When the circuit breaker is open, requests to the subgraph will be short-circuited and an error will be returned to the client.
    /// The circuit breaker will be triggered based on the error rate of requests to the subgraph, and will attempt to reset after a certain timeout.
    pub circuit_breaker: Option<TrafficShapingSubgraphCircuitBreakerConfig>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct TrafficShapingExecutorGlobalConfig {
    /// Timeout for idle sockets being kept-alive.
    #[serde(
        default = "default_pool_idle_timeout",
        deserialize_with = "humantime_serde::deserialize",
        serialize_with = "humantime_serde::serialize"
    )]
    #[schemars(with = "String")]
    pub pool_idle_timeout: Duration,

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

    /// Circuit Breaker configuration for all subgraphs.
    /// When the circuit breaker is open, requests to the subgraph will be
    /// short-circuited and an error will be returned to the client.
    /// The circuit breaker will be triggered based on the error rate of requests to the subgraph, and will attempt to reset after a certain timeout.
    pub circuit_breaker: Option<TrafficShapingSubgraphCircuitBreakerConfig>,
}

fn default_subgraph_pool_idle_timeout() -> Option<Duration> {
    None
}

fn default_request_timeout() -> DurationOrExpression {
    DurationOrExpression::Duration(Duration::from_secs(30))
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(untagged)]
pub enum DurationOrExpression {
    /// A fixed duration, e.g., "5s" or "100ms".
    #[serde(
        deserialize_with = "humantime_serde::deserialize",
        serialize_with = "humantime_serde::serialize"
    )]
    #[schemars(with = "String")]
    Duration(Duration),
    /// A VRL expression that evaluates to a duration. The result can be an integer (milliseconds) or a duration string (e.g. "5s").
    Expression { expression: String },
}

impl Default for TrafficShapingExecutorGlobalConfig {
    fn default() -> Self {
        Self {
            pool_idle_timeout: default_pool_idle_timeout(),
            dedupe_enabled: default_dedupe_enabled(),
            request_timeout: default_request_timeout(),
            circuit_breaker: default_circuit_breaker_config(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct TrafficShapingRouterConfig {
    #[serde(default)]
    pub dedupe: TrafficShapingRouterDedupeConfig,

    /// Optional timeout configuration for incoming requests to the router.
    /// It starts from the moment the request is received by the router,
    /// and includes the entire processing of the request (validation, execution, etc.) until a response is sent back to the client.
    /// If a request takes longer than the specified duration, it will be aborted and a timeout error will be returned to the client.
    #[serde(
        default = "default_router_request_timeout",
        deserialize_with = "humantime_serde::deserialize",
        serialize_with = "humantime_serde::serialize"
    )]
    #[schemars(with = "String")]
    pub request_timeout: Duration,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct TrafficShapingRouterDedupeConfig {
    /// Enables/disables in-flight request deduplication at the router endpoint level.
    ///
    /// When enabled, identical incoming GraphQL query requests that are processed at the same time
    /// share the same in-flight execution result.
    #[serde(default = "default_router_dedupe_enabled")]
    pub enabled: bool,

    /// Header configuration participating in the dedupe key.
    ///
    /// Accepted forms:
    /// - `all`
    /// - `none`
    /// - `{ include: ["authorization", "cookie"] }`
    ///
    /// Header names are case-insensitive and validated as standard HTTP header names.
    #[serde(default)]
    pub headers: TrafficShapingRouterDedupeHeadersConfig,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum TrafficShapingRouterDedupeHeadersKeyword {
    #[default]
    All,
    None,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(untagged)]
pub enum TrafficShapingRouterDedupeHeadersConfig {
    Keyword(TrafficShapingRouterDedupeHeadersKeyword),
    Include { include: Vec<HttpHeaderName> },
}

impl Default for TrafficShapingRouterDedupeHeadersConfig {
    fn default() -> Self {
        Self::Keyword(TrafficShapingRouterDedupeHeadersKeyword::All)
    }
}

impl Default for TrafficShapingRouterDedupeConfig {
    fn default() -> Self {
        Self {
            enabled: default_router_dedupe_enabled(),
            headers: Default::default(),
        }
    }
}

fn default_router_request_timeout() -> Duration {
    Duration::from_secs(60)
}

impl Default for TrafficShapingRouterConfig {
    fn default() -> Self {
        Self {
            dedupe: Default::default(),
            request_timeout: default_router_request_timeout(),
        }
    }
}

fn default_circuit_breaker_config() -> Option<TrafficShapingSubgraphCircuitBreakerConfig> {
    None
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct TrafficShapingSubgraphCircuitBreakerConfig {
    /// Enable or disable the circuit breaker for the subgraph.
    /// Default: false (circuit breaker is disabled)
    #[serde(default = "default_circuit_breaker_enabled")]
    pub enabled: bool,
    /// Percentage after what the circuit breaker should kick in.
    /// Default: 50%
    #[serde(default)]
    #[schemars(with = "String")]
    pub error_threshold: Option<Percentage>,
    /// Count of requests before starting evaluating.
    /// Default: 5
    #[serde(default)]
    pub volume_threshold: Option<usize>,
    /// The duration after which the circuit breaker will attempt to retry sending requests to the subgraph.
    /// Default: 30s
    #[serde(
        default,
        deserialize_with = "humantime_serde::deserialize",
        serialize_with = "humantime_serde::serialize"
    )]
    #[schemars(with = "String")]
    pub reset_timeout: Option<Duration>,
}

fn default_circuit_breaker_enabled() -> bool {
    false
}
