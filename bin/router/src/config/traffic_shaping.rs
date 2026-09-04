use std::{borrow::Cow, collections::HashMap, fmt, time::Duration};

use http::StatusCode;
use human_size::Size;
use schemars::{json_schema, JsonSchema, Schema, SchemaGenerator};
use serde::{de, Deserialize, Serialize};

use crate::config::primitives::{
    file_path::FilePath, http_header::HttpHeaderName, percentage::Percentage,
    single_or_multiple::SingleOrMultiple,
};

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
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

/// Traffic shaping that belongs to one supergraph's subgraph executors.
#[derive(Clone)]
#[non_exhaustive]
pub struct SupergraphTrafficShapingConfig {
    pub all: TrafficShapingExecutorGlobalConfig,
    pub subgraphs: HashMap<String, TrafficShapingExecutorSubgraphConfig>,
    pub max_connections_per_host: usize,
}

impl SupergraphTrafficShapingConfig {
    /// Returns whether WebSocket connections should be reused for a subgraph.
    ///
    /// A per-subgraph value takes precedence over the value configured in `all`.
    pub fn websocket_reuse_connections(&self, subgraph_name: &str) -> bool {
        self.subgraphs
            .get(subgraph_name)
            .and_then(|config| config.websocket.as_ref())
            .and_then(|config| config.reuse_connections)
            .or(self.all.websocket.reuse_connections)
            .unwrap_or(true)
    }

    /// Returns how queries and mutations should be transported to a WebSocket-enabled subgraph.
    ///
    /// A per-subgraph value takes precedence over the value configured in `all`.
    pub fn websocket_execute_mode(&self, subgraph_name: &str) -> WebSocketExecuteMode {
        self.subgraphs
            .get(subgraph_name)
            .and_then(|config| config.websocket.as_ref())
            .and_then(|config| config.execute_mode)
            .or(self.all.websocket.execute_mode)
            .unwrap_or_default()
    }

    /// Returns the idle timeout used by HTTP and pooled WebSocket connections for a subgraph.
    ///
    /// A per-subgraph value takes precedence over `traffic_shaping.all.pool_idle_timeout`.
    pub fn pool_idle_timeout(&self, subgraph_name: &str) -> Duration {
        self.subgraphs
            .get(subgraph_name)
            .and_then(|config| config.pool_idle_timeout)
            .unwrap_or(self.all.pool_idle_timeout)
    }

    /// Returns the resolved compression config for a subgraph.
    ///
    /// A per-subgraph `compression` block fully replaces `traffic_shaping.all.compression`
    /// (unlike `websocket`, fields aren't merged - `request.enabled` and `request.algorithm`
    /// always come from the same level).
    pub fn subgraph_compression(
        &self,
        subgraph_name: &str,
    ) -> TrafficShapingSubgraphCompressionConfig {
        self.subgraphs
            .get(subgraph_name)
            .and_then(|config| config.compression)
            .unwrap_or(self.all.compression)
    }

    /// Returns whether any configured traffic-shaping rule can reuse WebSocket connections.
    ///
    /// This is used to avoid connection fingerprinting when pooling is disabled globally and for
    /// every explicit subgraph override.
    pub fn any_websocket_connection_reuse_enabled(&self) -> bool {
        self.all.websocket.reuse_connections.unwrap_or(true)
            || self.subgraphs.values().any(|config| {
                config
                    .websocket
                    .as_ref()
                    .and_then(|websocket| websocket.reuse_connections)
                    .unwrap_or(false)
            })
    }
}

impl Default for SupergraphTrafficShapingConfig {
    fn default() -> Self {
        Self {
            all: TrafficShapingExecutorGlobalConfig::default(),
            subgraphs: HashMap::new(),
            max_connections_per_host: default_max_connections_per_host(),
        }
    }
}

impl From<&TrafficShapingConfig> for SupergraphTrafficShapingConfig {
    fn from(config: &TrafficShapingConfig) -> Self {
        Self {
            all: config.all.clone(),
            subgraphs: config.subgraphs.clone(),
            max_connections_per_host: config.max_connections_per_host,
        }
    }
}

fn default_max_connections_per_host() -> usize {
    100
}

fn default_pool_idle_timeout() -> Duration {
    Duration::from_secs(50)
}

fn default_subgraph_pool_idle_timeout() -> Option<Duration> {
    None
}

fn default_dedupe_enabled() -> bool {
    true
}

fn default_router_dedupe_enabled() -> bool {
    false
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct TrafficShapingExecutorSubgraphConfig {
    /// Overrides how long idle pooled connections for this subgraph remain available for reuse.
    ///
    /// This timeout applies to both the HTTP connection pool and pooled WebSocket connections for
    /// this subgraph. Active WebSocket operations are never expired by this setting. Their idle
    /// timer starts only after the last operation on the pooled connection finishes.
    ///
    /// When omitted, `traffic_shaping.all.pool_idle_timeout` is used.
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

    #[serde(skip_serializing_if = "Option::is_none")]
    pub tls: Option<ClientTLSConfig>,

    /// Forces HTTP/2 for requests to subgraphs.
    ///
    /// For plain HTTP, it will use HTTP/2 cleartext (h2c).
    /// For HTTPS, it also requires HTTP/2.
    /// This will make the subgraph requests never fall back to HTTP/1.1,
    /// and will fail if the subgraph doesn't support HTTP/2.
    pub allow_only_http2: Option<bool>,

    /// When enabled, forwards client operation name to the selected subgraph.
    /// The operation name will include fetch node id and operation name from the client request.
    /// Format: <Client Operation Name>__<Fetch Node ID>
    ///
    /// This setting takes precedence over the value set in `all` section.
    #[serde(default)]
    pub forward_operation_name: Option<bool>,

    /// Overrides WebSocket connection reuse and execution behavior for this subgraph.
    ///
    /// Omitted fields inherit their values from `traffic_shaping.all.websocket`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub websocket: Option<TrafficShapingWebSocketConfig>,

    /// Overrides compression of request bodies sent to this subgraph.
    ///
    /// When omitted, `traffic_shaping.all.compression` is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compression: Option<TrafficShapingSubgraphCompressionConfig>,
}

/// Controls which transport queries and mutations use for WebSocket-enabled subgraphs.
///
/// This setting does not select the subscription protocol. WebSocket support and endpoint paths
/// are declared under `subscriptions.websocket`. It only controls whether ordinary query and
/// mutation fetches may use those declared WebSocket endpoints.
#[derive(Debug, Default, Deserialize, Serialize, JsonSchema, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum WebSocketExecuteMode {
    /// Always executes queries and mutations over HTTP.
    ///
    /// WebSocket subscriptions are unaffected and may still reuse connections when
    /// `reuse_connections` is enabled.
    #[default]
    Http,
    /// Uses an already initialized matching WebSocket connection when one exists.
    ///
    /// A missing or still-connecting pool entry immediately falls back to HTTP. Queries and
    /// mutations never create or wait for a WebSocket in this mode.
    ReuseExisting,
    /// Executes queries and mutations over WebSocket.
    ///
    /// With `reuse_connections` enabled, a missing connection is initialized lazily and
    /// concurrent operations wait for the same initialization. With reuse disabled, every
    /// operation creates its own WebSocket connection.
    Websocket,
}

/// WebSocket traffic-shaping behavior used by both `all` and per-subgraph configuration.
///
/// Fields are optional so a subgraph can override one behavior while inheriting the other from
/// `traffic_shaping.all.websocket`. Fields omitted from `all` use their documented defaults.
#[derive(Debug, Default, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct TrafficShapingWebSocketConfig {
    /// Enables multiplexing operations over matching initialized WebSocket connections.
    ///
    /// When enabled, subscriptions retain initialized connections in a pool and can share one
    /// physical connection. Queries and mutations use that pool according to `execute_mode`.
    /// When disabled, each WebSocket operation owns a dedicated connection.
    ///
    /// A pooled connection is discovered using two values:
    ///
    /// 1. The resolved WebSocket endpoint. Endpoint overrides and expressions are evaluated first,
    ///    then the configured WebSocket path is applied. Connections to different resolved
    ///    endpoints are never considered a match.
    /// 2. The inbound connection fingerprint. This fingerprint contains the inbound HTTP method,
    ///    request path, selected inbound headers, and schema checksum. It deliberately excludes the
    ///    GraphQL operation, variables, and extensions, allowing different operations from the
    ///    same connection identity to share one physical WebSocket.
    ///
    /// Header selection uses `traffic_shaping.router.dedupe.headers`, even when router request
    /// deduplication itself is disabled. The default is `all`. If a custom header selection is
    /// configured, it must include every inbound header that can change the authentication,
    /// authorization, cookie, or tenant identity sent in `connection_init`. Excluding such a
    /// header can make requests with different identities appear to match and reuse the same
    /// authenticated connection.
    ///
    /// A subscription can create the matching pool entry. With `execute_mode: reuse_existing`, a
    /// query or mutation uses the connection only after it is fully initialized. A missing entry
    /// or one still waiting for `connection_ack` falls back to HTTP. With
    /// `execute_mode: websocket`, queries and mutations may create a missing entry or join an
    /// initialization already in progress.
    ///
    /// For example, the following configuration lets subscriptions create shared connections and
    /// lets queries and mutations reuse them when available:
    ///
    /// ```yaml
    /// traffic_shaping:
    ///   all:
    ///     websocket:
    ///       reuse_connections: true
    ///       execute_mode: reuse_existing
    ///   router:
    ///     dedupe:
    ///       headers:
    ///         include: [authorization, cookie, x-tenant]
    /// ```
    ///
    /// The following keeps reuse enabled globally but gives one subgraph a dedicated connection
    /// for every WebSocket operation:
    ///
    /// ```yaml
    /// traffic_shaping:
    ///   all:
    ///     websocket:
    ///       reuse_connections: true
    ///   subgraphs:
    ///     payments:
    ///       websocket:
    ///         reuse_connections: false
    /// ```
    ///
    /// Idle pooled connections use the effective `pool_idle_timeout` from traffic shaping. At the
    /// `all` level `reuse_connections` defaults to `true`. At the per-subgraph level omission
    /// inherits the `all` value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reuse_connections: Option<bool>,

    /// Controls whether queries and mutations use WebSocket-enabled subgraphs.
    ///
    /// The default is `http`, preserving HTTP execution unless WebSocket execution is explicitly
    /// enabled. Subscriptions continue to use the protocol selected under `subscriptions`.
    /// At the per-subgraph level omission inherits the `all` value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execute_mode: Option<WebSocketExecuteMode>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct TrafficShapingExecutorGlobalConfig {
    /// Controls how long idle pooled connections remain available for reuse by default.
    ///
    /// This timeout applies to both HTTP connection pools and pooled WebSocket connections for
    /// every subgraph that does not provide an override. Active WebSocket operations are never
    /// expired by this setting. Their idle timer starts only after the last operation on the
    /// pooled connection finishes.
    ///
    /// Defaults to 50 seconds.
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

    #[serde(skip_serializing_if = "Option::is_none")]
    pub tls: Option<ClientTLSConfig>,

    /// Forces HTTP/2 for requests to subgraphs.
    ///
    /// For plain HTTP, it will use HTTP/2 cleartext (h2c).
    /// For HTTPS, it also requires HTTP/2.
    /// This will make the subgraph requests never fall back to HTTP/1.1,
    /// and will fail if the subgraph doesn't support HTTP/2.
    #[serde(default)]
    pub allow_only_http2: bool,

    /// When enabled, forwards client operation name to subgraphs.
    /// The operation name will fetch node id and operation name from the client request.
    /// Format: <Client Operation Name>__<Fetch Node ID>
    #[serde(default)]
    pub forward_operation_name: bool,

    /// Default WebSocket connection reuse and execution behavior for subgraphs.
    ///
    /// Per-subgraph values under `traffic_shaping.subgraphs.<name>.websocket` take precedence.
    #[serde(default)]
    pub websocket: TrafficShapingWebSocketConfig,

    /// Default compression of request bodies sent to subgraphs.
    ///
    /// Per-subgraph values under `traffic_shaping.subgraphs.<name>.compression` take precedence.
    ///
    /// This only controls the router -> subgraph direction. Subgraph responses are always
    /// transparently decompressed when they carry a recognized `Content-Encoding` (`gzip`,
    /// `deflate`, `br`, `zstd`) - there's no configuration for that direction, matching Apollo
    /// Router, Cosmo, and Hive Gateway's behavior here.
    #[serde(default)]
    pub compression: TrafficShapingSubgraphCompressionConfig,
}

fn default_request_timeout() -> DurationOrExpression {
    DurationOrExpression::Duration(Duration::from_secs(30))
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(untagged)]
#[non_exhaustive]
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
            tls: None,
            allow_only_http2: false,
            forward_operation_name: false,
            websocket: Default::default(),
            compression: Default::default(),
        }
    }
}

/// Compression of traffic between the router and a subgraph.
///
/// Subgraph responses are always transparently decompressed when they carry a recognized
/// `Content-Encoding` - `request` is the only configurable direction for now (compressing
/// outbound requests unconditionally could break a subgraph that doesn't decompress, so it's
/// opt-in). Nested under its own key so a `response` section (e.g. tuning for the always-on
/// decompression) can be added later without a breaking shape change.
#[derive(Debug, Default, Deserialize, Serialize, JsonSchema, Clone, Copy)]
#[serde(deny_unknown_fields)]
pub struct TrafficShapingSubgraphCompressionConfig {
    /// Compresses request bodies sent to the subgraph.
    #[serde(default)]
    pub request: TrafficShapingSubgraphRequestCompressionConfig,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, Copy)]
#[serde(deny_unknown_fields)]
pub struct TrafficShapingSubgraphRequestCompressionConfig {
    /// Enables/disables compressing request bodies sent to the subgraph.
    #[serde(default)]
    pub enabled: bool,

    /// The algorithm to compress with, and its tuning for `br`/`zstd`. Only relevant when
    /// `enabled` is `true`.
    ///
    /// Unlike the client-facing `traffic_shaping.router.compression.response.algorithms`,
    /// this is a single choice, not an allow-list: the router doesn't negotiate with the
    /// subgraph, it unilaterally picks one algorithm and sends it.
    #[serde(default = "default_subgraph_compression_algorithm")]
    pub algorithm: CompressionAlgorithmConfig,
}

impl Default for TrafficShapingSubgraphRequestCompressionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            algorithm: default_subgraph_compression_algorithm(),
        }
    }
}

fn default_subgraph_compression_algorithm() -> CompressionAlgorithmConfig {
    CompressionAlgorithmConfig::Gzip
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
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

    /// TLS configuration for the HTTP server.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tls: Option<ServerTLSConfig>,

    /// Maximum number of concurrent long-lived clients (WebSocket connections and HTTP streaming responses, e.g. subscriptions over SSE or multipart).
    /// Regular non-streaming requests are not counted toward this limit, even when their `Accept` header advertises support for streaming response formats.
    /// When the limit is reached, new WebSocket connections and HTTP subscription requests are rejected with 503.
    /// If both WebSockets and Subscriptions are disabled, this setting has no effect.
    #[serde(default = "default_max_long_lived_clients")]
    pub max_long_lived_clients: usize,

    /// How long the HTTP server waits for a follow-up request on an idle
    /// keep-alive connection before closing it.
    ///
    /// Defaults to 5 seconds, which matches ntex's own default.
    ///
    /// When the router sits behind a reverse proxy, set this above that proxy's
    /// idle timeout so the proxy closes first. Closing first from the router
    /// leaves the proxy holding a socket the server has already dropped, which
    /// then fails the next reused request.
    ///
    /// ```yaml
    /// traffic_shaping:
    ///   router:
    ///     keep_alive: 80s
    /// ```
    ///
    /// Can also be set via the `ROUTER_HTTP_KEEP_ALIVE` environment variable.
    #[serde(
        default = "http_server_keep_alive_default",
        deserialize_with = "humantime_serde::deserialize",
        serialize_with = "humantime_serde::serialize"
    )]
    #[schemars(with = "String")]
    pub keep_alive: Duration,

    /// HTTP compression between the client and the router: compressing responses sent to the
    /// client, and decompressing requests received from the client.
    ///
    /// This is unrelated to `traffic_shaping.all`/`traffic_shaping.subgraphs`, which control
    /// compression between the router and subgraphs.
    #[serde(default)]
    pub compression: TrafficShapingRouterCompressionConfig,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
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
    ///
    /// Note: `content-type` is part of the fingerprint when `headers` includes it (e.g. `all`).
    /// Since HTTP streaming clients send different `accept` headers than WebSocket clients,
    /// cross-transport deduplication for subscriptions only applies when `content-type` (and
    /// transport-specific headers) are excluded from the key. Configure `headers: none` or
    /// `headers: { include: [] }` (or exclude the relevant headers) to enable true cross-transport
    /// deduplication, where a WebSocket subscription and an SSE subscription with the same operation
    /// share a single upstream connection and the events are fanned out to both.
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
#[non_exhaustive]
pub enum TrafficShapingRouterDedupeHeadersKeyword {
    #[default]
    All,
    None,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(untagged)]
#[non_exhaustive]
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
            tls: None,
            max_long_lived_clients: default_max_long_lived_clients(),
            keep_alive: http_server_keep_alive_default(),
            compression: Default::default(),
        }
    }
}

/// HTTP compression settings for traffic between the client and the router.
#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
#[derive(Default)]
pub struct TrafficShapingRouterCompressionConfig {
    /// Compression of responses sent from the router to the client, negotiated via the
    /// client's `Accept-Encoding` header.
    #[serde(default)]
    pub response: TrafficShapingRouterResponseCompressionConfig,

    /// Decompression of requests sent from the client to the router, based on the
    /// client's `Content-Encoding` header.
    #[serde(default)]
    pub request: TrafficShapingRouterRequestCompressionConfig,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct TrafficShapingRouterResponseCompressionConfig {
    /// Enables/disables compressing responses sent to the client.
    #[serde(default = "default_compression_enabled")]
    pub enabled: bool,

    /// Algorithms the router is allowed to compress responses with, and their tuning.
    ///
    /// The router picks the entry from this list whose `kind` best matches the client's
    /// `Accept-Encoding` preference; a client asking for an algorithm not listed here
    /// gets an uncompressed response instead. The list's order only matters as a
    /// tie-breaker when the client's preferences don't disambiguate.
    #[serde(default = "default_response_compression_algorithms")]
    pub algorithms: Vec<CompressionAlgorithmConfig>,

    /// Responses smaller than this are sent uncompressed, since compressing small
    /// payloads costs more CPU than it saves in bytes transferred.
    #[serde(default = "default_compression_min_size")]
    #[schemars(with = "String")]
    pub min_size: Size,
}

impl Default for TrafficShapingRouterResponseCompressionConfig {
    fn default() -> Self {
        Self {
            enabled: default_compression_enabled(),
            algorithms: default_response_compression_algorithms(),
            min_size: default_compression_min_size(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct TrafficShapingRouterRequestCompressionConfig {
    /// Enables/disables decompressing requests received from the client.
    ///
    /// When disabled, a request carrying a `Content-Encoding` header is rejected with
    /// `415 Unsupported Media Type` instead of being decompressed.
    #[serde(default = "default_compression_enabled")]
    pub enabled: bool,

    /// Algorithms the router accepts in a client's `Content-Encoding` header.
    ///
    /// A request encoded with an algorithm not listed here is rejected with
    /// `415 Unsupported Media Type`.
    #[serde(default = "default_request_compression_algorithms")]
    pub algorithms: Vec<CompressionAlgorithm>,
}

impl Default for TrafficShapingRouterRequestCompressionConfig {
    fn default() -> Self {
        Self {
            enabled: default_compression_enabled(),
            algorithms: default_request_compression_algorithms(),
        }
    }
}

/// A content-coding algorithm identified by its `Accept-Encoding`/`Content-Encoding` token.
///
/// Used for `compression.request.algorithms`: decompression has no per-algorithm tuning,
/// so a plain allow-list is all that's needed there. See [`CompressionAlgorithmConfig`]
/// for the response side, where `br`/`zstd` carry inline tuning.
#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CompressionAlgorithm {
    Gzip,
    Deflate,
    Br,
    Zstd,
}

impl CompressionAlgorithm {
    /// The `Accept-Encoding`/`Content-Encoding` token this variant matches.
    pub fn token(&self) -> &'static str {
        match self {
            Self::Gzip => "gzip",
            Self::Deflate => "deflate",
            Self::Br => "br",
            Self::Zstd => "zstd",
        }
    }

    /// Parses a single `Content-Encoding` token (case-insensitive). Returns `None` for
    /// anything that isn't one of the four known tokens - callers decide what that means
    /// (identity, unsupported, or a stacked/malformed value).
    pub fn from_token(token: &str) -> Option<Self> {
        match token {
            t if t.eq_ignore_ascii_case("gzip") => Some(Self::Gzip),
            t if t.eq_ignore_ascii_case("deflate") => Some(Self::Deflate),
            t if t.eq_ignore_ascii_case("br") => Some(Self::Br),
            t if t.eq_ignore_ascii_case("zstd") => Some(Self::Zstd),
            _ => None,
        }
    }
}

/// One entry of `compression.response.algorithms`: an allowed algorithm plus, for
/// algorithms that have one, its tuning - co-located so enabling an algorithm and
/// configuring it happen in the same place instead of a separate lookup table.
#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, Copy, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CompressionAlgorithmConfig {
    Gzip,
    Deflate,
    Br(BrotliCompressionConfig),
    Zstd(ZstdCompressionConfig),
}

impl CompressionAlgorithmConfig {
    /// The `Accept-Encoding`/`Content-Encoding` token this entry matches/produces.
    pub fn token(&self) -> &'static str {
        match self {
            Self::Gzip => "gzip",
            Self::Deflate => "deflate",
            Self::Br(_) => "br",
            Self::Zstd(_) => "zstd",
        }
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, Copy, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BrotliCompressionConfig {
    /// Brotli quality level, from `0` (fastest, worst ratio) to `11` (slowest, best ratio).
    ///
    /// Defaults well below the maximum: quality `11` is slow enough to become a bottleneck
    /// for dynamically generated GraphQL responses, where CPU time usually matters more
    /// than squeezing out the last few percent of size.
    #[serde(
        default = "default_brotli_quality",
        deserialize_with = "deserialize_brotli_quality"
    )]
    #[schemars(range(min = 0, max = 11))]
    pub quality: u8,
}

fn deserialize_brotli_quality<'de, D>(deserializer: D) -> Result<u8, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = u8::deserialize(deserializer)?;
    if value > 11 {
        return Err(de::Error::custom(format!(
            "brotli quality must be between 0 and 11, got: {value}"
        )));
    }
    Ok(value)
}

impl Default for BrotliCompressionConfig {
    fn default() -> Self {
        Self {
            quality: default_brotli_quality(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, Copy, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ZstdCompressionConfig {
    /// Zstandard compression level, from `1` (fastest, worst ratio) to `22` (slowest, best ratio).
    ///
    /// Defaults to zstd's own standard default, which already gives a good balance of
    /// speed and ratio for dynamically generated GraphQL responses.
    #[serde(
        default = "default_zstd_level",
        deserialize_with = "deserialize_zstd_level"
    )]
    #[schemars(range(min = 1, max = 22))]
    pub level: i32,
}

fn deserialize_zstd_level<'de, D>(deserializer: D) -> Result<i32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = i32::deserialize(deserializer)?;
    if !(1..=22).contains(&value) {
        return Err(de::Error::custom(format!(
            "zstd level must be between 1 and 22, got: {value}"
        )));
    }
    Ok(value)
}

impl Default for ZstdCompressionConfig {
    fn default() -> Self {
        Self {
            level: default_zstd_level(),
        }
    }
}

fn default_compression_enabled() -> bool {
    true
}

fn default_response_compression_algorithms() -> Vec<CompressionAlgorithmConfig> {
    vec![
        CompressionAlgorithmConfig::Gzip,
        CompressionAlgorithmConfig::Zstd(ZstdCompressionConfig::default()),
        CompressionAlgorithmConfig::Br(BrotliCompressionConfig::default()),
        CompressionAlgorithmConfig::Deflate,
    ]
}

fn default_request_compression_algorithms() -> Vec<CompressionAlgorithm> {
    vec![
        CompressionAlgorithm::Gzip,
        CompressionAlgorithm::Zstd,
        CompressionAlgorithm::Br,
        CompressionAlgorithm::Deflate,
    ]
}

fn default_compression_min_size() -> Size {
    "1KiB"
        .parse()
        .expect("Default value for 'traffic_shaping.router.compression.response.min_size' should be a valid human-readable size")
}

fn default_brotli_quality() -> u8 {
    5
}

fn default_zstd_level() -> i32 {
    // matches zstd's own `ZSTD_CLEVEL_DEFAULT`
    3
}

/// Matches ntex's own default HTTP/1 keep-alive, so configurations that omit
/// `keep_alive` keep their existing behaviour.
fn http_server_keep_alive_default() -> Duration {
    Duration::from_secs(5)
}

fn default_circuit_breaker_config() -> Option<TrafficShapingSubgraphCircuitBreakerConfig> {
    None
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ServerTLSConfig {
    pub cert_file: SingleOrMultiple<FilePath>,
    pub key_file: FilePath,
    pub client_auth: Option<ServerClientAuthConfig>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct TrafficShapingSubgraphCircuitBreakerConfig {
    /// Enable or disable the circuit breaker for the subgraph.
    /// Default: false (circuit breaker is disabled)
    ///
    /// When unset on a subgraph-level configuration, the value falls back
    /// to the value defined in the global (`all`) circuit breaker
    /// configuration.
    #[serde(default)]
    pub enabled: Option<bool>,
    /// Percentage after what the circuit breaker should kick in.
    /// Default: 50%
    #[serde(default)]
    #[schemars(with = "String")]
    pub error_threshold: Option<Percentage>,
    /// Size of the rolling sample used to decide whether the breaker
    /// should open while closed. The breaker fills this sample with the
    /// outcomes of the last `volume_threshold` requests; the next request
    /// after the sample is full is the one whose result is evaluated
    /// against `error_threshold`. In practice the breaker can trip only
    /// after at least `volume_threshold + 1` requests have been observed.
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
    /// Size of the rolling sample of probe requests collected while the
    /// breaker is in the half-open state after `reset_timeout` elapses.
    /// The breaker fills this sample first; the next probe after the
    /// sample is full is the one whose result is evaluated against
    /// `error_threshold` to decide whether to transition back to `closed`
    /// (resuming normal traffic) or to `open` (waiting for another
    /// `reset_timeout` window). In practice at least
    /// `half_open_attempts + 1` probes pass through before the breaker
    /// can transition.
    ///
    /// Lower values make recovery faster but more aggressive; higher
    /// values gather more samples before re-closing the circuit.
    ///
    /// Default: 10
    #[serde(default)]
    pub half_open_attempts: Option<usize>,
    /// HTTP status codes returned by the subgraph that should be counted as
    /// failures by the circuit breaker.
    ///
    /// Each entry can be either an exact status code (integer or string,
    /// e.g. `503` or `"503"`) or a wildcard pattern in one of these forms:
    ///
    /// - `"5xx"` - matches every 500-599 status (`[1-5]xx` accepted),
    /// - `"50x"` - matches every 500-509 status (`[1-5][0-9]x` accepted).
    ///
    /// Wildcards are case-insensitive (`"5XX"` works too). Patterns can be
    /// freely mixed with exact codes in the same list, for example:
    ///
    /// ```yaml
    /// error_status_codes: [501, "5xx", "52x"]
    /// ```
    ///
    /// Only responses whose status code matches at least one entry in this
    /// list are recorded as failures by the circuit breaker. Responses with
    /// any other status code are treated as successes from the breaker's
    /// point of view.
    ///
    /// Default: `[500, 502, 503, 504]`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_status_codes: Option<Vec<StatusCodeMatcher>>,
}

/// Matches an HTTP status code either exactly or via a wildcard pattern.
///
/// See [`TrafficShapingSubgraphCircuitBreakerConfig::error_status_codes`] for
/// the accepted syntax.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum StatusCodeMatcher {
    /// A single exact HTTP status code, e.g. `503`.
    Exact(StatusCode),
    /// A `Nxx` wildcard matching every status in the `N00..=N99` range.
    /// `N` is stored as `1..=5` (e.g. `5` for `"5xx"`).
    Hundreds(u8),
    /// A `NNx` wildcard matching every status in the `NN0..=NN9` range.
    /// The prefix is stored as `10..=59` (e.g. `50` for `"50x"`).
    Tens(u16),
}

impl StatusCodeMatcher {
    /// Returns `true` if the given status code is covered by this matcher.
    pub fn matches(&self, status: StatusCode) -> bool {
        match self {
            StatusCodeMatcher::Exact(code) => *code == status,
            StatusCodeMatcher::Hundreds(n) => {
                let lower = u16::from(*n) * 100;
                let value = status.as_u16();
                value >= lower && value <= lower + 99
            }
            StatusCodeMatcher::Tens(n) => {
                let lower = *n * 10;
                let value = status.as_u16();
                value >= lower && value <= lower + 9
            }
        }
    }

    fn parse_str(input: &str) -> Result<Self, String> {
        let lower = input.to_ascii_lowercase();
        if lower.len() == 3 {
            if lower.ends_with("xx") {
                let n: u8 = lower[..1].parse().map_err(|_| {
                    format!("invalid wildcard status code pattern '{input}': expected '[1-5]xx'")
                })?;
                if !(1..=5).contains(&n) {
                    return Err(format!(
                        "invalid wildcard status code pattern '{input}': hundreds digit must be in 1-5"
                    ));
                }
                return Ok(StatusCodeMatcher::Hundreds(n));
            }
            if lower.ends_with('x') {
                let n: u16 = lower[..2].parse().map_err(|_| {
                    format!(
                        "invalid wildcard status code pattern '{input}': expected '[1-5][0-9]x'"
                    )
                })?;
                if !(10..=59).contains(&n) {
                    return Err(format!(
                        "invalid wildcard status code pattern '{input}': tens prefix must be in 10-59"
                    ));
                }
                return Ok(StatusCodeMatcher::Tens(n));
            }
        }

        let code: u16 = input
            .parse()
            .map_err(|_| format!("invalid HTTP status code or wildcard pattern '{input}'"))?;
        StatusCode::from_u16(code)
            .map(StatusCodeMatcher::Exact)
            .map_err(|_| format!("invalid HTTP status code '{input}'"))
    }
}

impl Serialize for StatusCodeMatcher {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            StatusCodeMatcher::Exact(code) => serializer.serialize_u16(code.as_u16()),
            StatusCodeMatcher::Hundreds(n) => serializer.serialize_str(&format!("{n}xx")),
            StatusCodeMatcher::Tens(n) => serializer.serialize_str(&format!("{n}x")),
        }
    }
}

impl<'de> Deserialize<'de> for StatusCodeMatcher {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;

        impl<'de> de::Visitor<'de> for Visitor {
            type Value = StatusCodeMatcher;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str(
                    "an HTTP status code (integer 100-599) or a wildcard pattern like \"5xx\" or \"50x\"",
                )
            }

            fn visit_u64<E: de::Error>(self, value: u64) -> Result<Self::Value, E> {
                let code = u16::try_from(value)
                    .map_err(|_| E::custom(format!("invalid HTTP status code: {value}")))?;
                StatusCode::from_u16(code)
                    .map(StatusCodeMatcher::Exact)
                    .map_err(|_| E::custom(format!("invalid HTTP status code: {value}")))
            }

            fn visit_i64<E: de::Error>(self, value: i64) -> Result<Self::Value, E> {
                let value: u64 = value
                    .try_into()
                    .map_err(|_| E::custom(format!("invalid HTTP status code: {value}")))?;
                self.visit_u64(value)
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
                StatusCodeMatcher::parse_str(value).map_err(E::custom)
            }
        }

        deserializer.deserialize_any(Visitor)
    }
}

impl JsonSchema for StatusCodeMatcher {
    fn schema_name() -> Cow<'static, str> {
        "StatusCodeMatcher".into()
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "Either an exact HTTP status code (integer 100-599 or its string form, e.g. 503) or a wildcard pattern: '[1-5]xx' (e.g. '5xx') or '[1-5][0-9]x' (e.g. '50x'). Case-insensitive.",
            "oneOf": [
                {
                    "type": "integer",
                    "minimum": 100,
                    "maximum": 599
                },
                {
                    "type": "string",
                    "pattern": "^(?:[1-5][0-9][0-9]|[1-5][xX][xX]|[1-5][0-9][xX])$"
                }
            ]
        })
    }

    fn inline_schema() -> bool {
        true
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ServerClientAuthConfig {
    pub cert_file: SingleOrMultiple<FilePath>,
    #[serde(default)]
    pub required: Option<bool>,
}

#[derive(Default, Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ClientTLSConfig {
    pub cert_file: Option<SingleOrMultiple<FilePath>>,
    pub client_auth: Option<ClientAuthConfig>,
    #[serde(default)]
    pub insecure_skip_ca_verification: bool,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ClientAuthConfig {
    pub cert_file: SingleOrMultiple<FilePath>,
    pub key_file: FilePath,
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{SupergraphTrafficShapingConfig, TrafficShapingConfig, TrafficShapingRouterConfig};

    #[test]
    fn keep_alive_defaults_to_five_seconds() {
        let config: TrafficShapingRouterConfig =
            serde_json::from_str("{}").expect("empty config should deserialize");

        assert_eq!(config.keep_alive, Duration::from_secs(5));
    }

    #[test]
    fn keep_alive_accepts_human_readable_durations() {
        let config: TrafficShapingRouterConfig = serde_json::from_str(r#"{ "keep_alive": "80s" }"#)
            .expect("human readable duration should deserialize");

        assert_eq!(config.keep_alive, Duration::from_secs(80));
    }

    #[test]
    fn keep_alive_serializes_back_to_a_duration_string() {
        let config = TrafficShapingRouterConfig {
            keep_alive: Duration::from_secs(80),
            ..Default::default()
        };

        let serialized = serde_json::to_value(&config)
            .expect("config should serialize")
            .get("keep_alive")
            .cloned();

        assert_eq!(serialized, Some(serde_json::json!("1m 20s")));
    }

    #[test]
    fn compression_defaults_are_enabled_with_gzip_br_deflate() {
        let config: TrafficShapingRouterConfig =
            serde_json::from_str("{}").expect("empty config should deserialize");

        assert!(config.compression.response.enabled);
        assert_eq!(
            config.compression.response.algorithms,
            vec![
                super::CompressionAlgorithmConfig::Gzip,
                super::CompressionAlgorithmConfig::Zstd(super::ZstdCompressionConfig { level: 3 }),
                super::CompressionAlgorithmConfig::Br(super::BrotliCompressionConfig {
                    quality: 5
                }),
                super::CompressionAlgorithmConfig::Deflate,
            ]
        );

        assert!(config.compression.request.enabled);
        assert_eq!(
            config.compression.request.algorithms,
            vec![
                super::CompressionAlgorithm::Gzip,
                super::CompressionAlgorithm::Zstd,
                super::CompressionAlgorithm::Br,
                super::CompressionAlgorithm::Deflate,
            ]
        );
    }

    #[test]
    fn compression_min_size_defaults_to_one_kibibyte() {
        let config: TrafficShapingRouterConfig =
            serde_json::from_str("{}").expect("empty config should deserialize");

        assert_eq!(
            config.compression.response.min_size.to_bytes(),
            1024,
            "default min_size should be 1KiB"
        );
    }

    #[test]
    fn compression_algorithm_tokens_match_http_header_casing() {
        // `gzip`/`deflate`/`br` are parsed directly from `Accept-Encoding`/`Content-Encoding`
        // values, so the config's on-the-wire spelling must match those tokens exactly.
        let algorithms: Vec<super::CompressionAlgorithm> =
            serde_json::from_str(r#"["gzip", "deflate", "br", "zstd"]"#)
                .expect("lowercase algorithm tokens should deserialize");

        assert_eq!(
            algorithms,
            vec![
                super::CompressionAlgorithm::Gzip,
                super::CompressionAlgorithm::Deflate,
                super::CompressionAlgorithm::Br,
                super::CompressionAlgorithm::Zstd,
            ]
        );
    }

    #[test]
    fn compression_response_algorithms_co_locate_tuning_with_kind() {
        let config: TrafficShapingRouterConfig = serde_json::from_str(
            r#"{
                "compression": {
                    "response": {
                        "algorithms": [
                            { "kind": "gzip" },
                            { "kind": "br", "quality": 9 },
                            { "kind": "zstd", "level": 10 },
                            { "kind": "deflate" }
                        ]
                    }
                }
            }"#,
        )
        .expect("kind-tagged algorithms list should deserialize");

        assert_eq!(
            config.compression.response.algorithms,
            vec![
                super::CompressionAlgorithmConfig::Gzip,
                super::CompressionAlgorithmConfig::Br(super::BrotliCompressionConfig {
                    quality: 9
                }),
                super::CompressionAlgorithmConfig::Zstd(super::ZstdCompressionConfig { level: 10 }),
                super::CompressionAlgorithmConfig::Deflate,
            ]
        );
    }

    #[test]
    fn compression_response_algorithm_tuning_defaults_when_omitted() {
        let config: TrafficShapingRouterConfig = serde_json::from_str(
            r#"{ "compression": { "response": { "algorithms": [{ "kind": "br" }] } } }"#,
        )
        .expect("kind without tuning fields should deserialize using defaults");

        assert_eq!(
            config.compression.response.algorithms,
            vec![super::CompressionAlgorithmConfig::Br(
                super::BrotliCompressionConfig { quality: 5 }
            )]
        );
    }

    #[test]
    fn compression_rejects_brotli_quality_above_eleven() {
        let result: Result<TrafficShapingRouterConfig, _> = serde_json::from_str(
            r#"{ "compression": { "response": { "algorithms": [{ "kind": "br", "quality": 12 }] } } }"#,
        );

        assert!(
            result.is_err(),
            "brotli quality above 11 should be rejected at config load time"
        );
    }

    #[test]
    fn compression_accepts_brotli_quality_at_the_boundaries() {
        for quality in [0, 11] {
            let config: TrafficShapingRouterConfig = serde_json::from_str(&format!(
                r#"{{ "compression": {{ "response": {{ "algorithms": [{{ "kind": "br", "quality": {quality} }}] }} }} }}"#
            ))
            .unwrap_or_else(|err| panic!("quality {quality} should be valid: {err}"));

            assert_eq!(
                config.compression.response.algorithms,
                vec![super::CompressionAlgorithmConfig::Br(
                    super::BrotliCompressionConfig { quality }
                )]
            );
        }
    }

    #[test]
    fn compression_rejects_zstd_level_outside_one_to_twenty_two() {
        for level in [0, 23, -1] {
            let result: Result<TrafficShapingRouterConfig, _> = serde_json::from_str(&format!(
                r#"{{ "compression": {{ "response": {{ "algorithms": [{{ "kind": "zstd", "level": {level} }}] }} }} }}"#
            ));

            assert!(
                result.is_err(),
                "zstd level {level} is outside 1-22 and should be rejected at config load time"
            );
        }
    }

    #[test]
    fn compression_accepts_zstd_level_at_the_boundaries() {
        for level in [1, 22] {
            let config: TrafficShapingRouterConfig = serde_json::from_str(&format!(
                r#"{{ "compression": {{ "response": {{ "algorithms": [{{ "kind": "zstd", "level": {level} }}] }} }} }}"#
            ))
            .unwrap_or_else(|err| panic!("level {level} should be valid: {err}"));

            assert_eq!(
                config.compression.response.algorithms,
                vec![super::CompressionAlgorithmConfig::Zstd(
                    super::ZstdCompressionConfig { level }
                )]
            );
        }
    }

    #[test]
    fn compression_rejects_unknown_fields() {
        let result: Result<TrafficShapingRouterConfig, _> =
            serde_json::from_str(r#"{ "compression": { "response": { "unknown_field": true } } }"#);

        assert!(
            result.is_err(),
            "unknown fields under compression.response should be rejected"
        );
    }

    #[test]
    fn subgraph_compression_defaults_to_disabled_gzip() {
        let config: TrafficShapingConfig =
            serde_json::from_str("{}").expect("empty config should deserialize");
        let resolved = SupergraphTrafficShapingConfig::from(&config);

        let compression = resolved.subgraph_compression("accounts");
        assert!(!compression.request.enabled);
        assert_eq!(
            compression.request.algorithm,
            super::CompressionAlgorithmConfig::Gzip
        );
    }

    #[test]
    fn subgraph_compression_inherits_from_all_when_not_overridden() {
        let config: TrafficShapingConfig = serde_json::from_str(
            r#"{ "all": { "compression": { "request": { "enabled": true, "algorithm": { "kind": "zstd", "level": 5 } } } } }"#,
        )
        .expect("config should deserialize");
        let resolved = SupergraphTrafficShapingConfig::from(&config);

        let compression = resolved.subgraph_compression("accounts");
        assert!(compression.request.enabled);
        assert_eq!(
            compression.request.algorithm,
            super::CompressionAlgorithmConfig::Zstd(super::ZstdCompressionConfig { level: 5 })
        );
    }

    #[test]
    fn subgraph_compression_override_replaces_all_entirely_rather_than_merging() {
        let config: TrafficShapingConfig = serde_json::from_str(
            r#"{
                "all": { "compression": { "request": { "enabled": true, "algorithm": { "kind": "gzip" } } } },
                "subgraphs": { "accounts": { "compression": { "request": { "enabled": false } } } }
            }"#,
        )
        .expect("config should deserialize");
        let resolved = SupergraphTrafficShapingConfig::from(&config);

        let accounts = resolved.subgraph_compression("accounts");
        assert!(
            !accounts.request.enabled,
            "subgraph override should replace, not merge with, `all`"
        );

        // an unrelated subgraph without its own override still inherits from `all`
        let products = resolved.subgraph_compression("products");
        assert!(products.request.enabled);
    }
}
