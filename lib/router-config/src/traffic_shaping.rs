use std::{collections::HashMap, time::Duration};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::primitives::http_header::HttpHeaderName;

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

    /// Maximum number of concurrent long-lived clients (WebSocket connections and HTTP streaming responses).
    /// Regular non-streaming requests are not counted toward this limit.
    /// When the limit is reached, new WebSocket and streaming HTTP requests are rejected with 503.
    /// If both WebSockets and Subscriptions are disabled, this setting has no effect.
    #[serde(default = "default_max_long_lived_clients")]
    pub max_long_lived_clients: usize,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct TrafficShapingRouterDedupeConfig {
    /// Enables/disables in-flight request and active subscriptions deduplication at the router level.
    ///
    /// When enabled, the router deduplicates both queries and subscriptions using the same
    /// fingerprint key (method, path, selected headers, schema checksum, normalized operation
    /// hash, variables, and extensions). The `headers` configuration below controls which
    /// headers participate in that key for all operation types.
    ///
    /// For queries, concurrent HTTP requests that produce the same fingerprint share a single
    /// in-flight execution - only the first one runs, and the rest wait for and receive the
    /// same result.
    ///
    /// For subscriptions, the mechanism is broadcast-based rather than request-sharing. The
    /// first client with a given fingerprint becomes the leader: it runs the upstream subscription
    /// and its events are fanned out through a broadcast channel backed by an active subscriptions
    /// registry. Any subsequent client that arrives with an identical fingerprint while that subscription
    /// is still active joins as a listener on the same broadcast channel instead of starting a new upstream
    /// connection. When all listeners have dropped and the leader finishes, the entry is removed from the
    /// registry.
    ///
    /// WebSocket connections participate in the same deduplication space as HTTP. Each
    /// subscribe message is processed with a synthetic request assembled from the WebSocket
    /// path and the headers derived from the `websocket.headers` config. The fingerprint is computed
    /// from those synthetic headers using the same header policy, so a subscription started over HTTP
    /// and an identical one started over WebSocket will deduplicate against each other.
    ///
    /// The deduplication is transport agnostic. A query over WebSocket would get deduplicated with an
    /// identical query over HTTP if they arrive at the same time and have the same fingerprint.
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

fn default_max_long_lived_clients() -> usize {
    128
}

impl Default for TrafficShapingRouterConfig {
    fn default() -> Self {
        Self {
            dedupe: Default::default(),
            request_timeout: default_router_request_timeout(),
            max_long_lived_clients: default_max_long_lived_clients(),
        }
    }
}
