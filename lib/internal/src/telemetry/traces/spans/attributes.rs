/// OpenTelemetry standard attributes
pub const OTEL_STATUS_CODE: &str = "otel.status_code";
pub const OTEL_KIND: &str = "otel.kind";

/// OpenTelemetry standard attributes for errors
pub const ERROR_TYPE: &str = "error.type";
pub const ERROR_MESSAGE: &str = "error.message";

/// HTTP attributes (OpenTelemetry Semantic Conventions)
pub const SERVER_ADDRESS: &str = "server.address";
pub const SERVER_PORT: &str = "server.port";
pub const URL_FULL: &str = "url.full";
pub const URL_PATH: &str = "url.path";
pub const URL_SCHEME: &str = "url.scheme";
pub const HTTP_REQUEST_BODY_SIZE: &str = "http.request.body.size";
pub const HTTP_REQUEST_METHOD: &str = "http.request.method";
pub const NETWORK_PROTOCOL_VERSION: &str = "network.protocol.version";
pub const USER_AGENT_ORIGINAL: &str = "user_agent.original";
pub const HTTP_RESPONSE_STATUS_CODE: &str = "http.response.status_code";
pub const HTTP_RESPONSE_BODY_SIZE: &str = "http.response.body.size";
pub const HTTP_ROUTE: &str = "http.route";

/// GraphQL Attributes
pub const GRAPHQL_OPERATION_NAME: &str = "graphql.operation.name";
pub const GRAPHQL_OPERATION_TYPE: &str = "graphql.operation.type";
pub const GRAPHQL_OPERATION_ID: &str = "graphql.operation.id";
pub const GRAPHQL_DOCUMENT_HASH: &str = "graphql.document.hash";
pub const GRAPHQL_DOCUMENT_TEXT: &str = "graphql.document.text";

/// Hive-specific attributes
pub const HIVE_KIND: &str = "hive.kind";
pub const HIVE_GRAPHQL_ERROR_COUNT: &str = "hive.graphql.error.count";
pub const HIVE_GRAPHQL_ERROR_CODES: &str = "hive.graphql.error.codes";
pub const HIVE_CLIENT_NAME: &str = "hive.client.name";
pub const HIVE_CLIENT_VERSION: &str = "hive.client.version";
pub const HIVE_GRAPHQL_OPERATION_HASH: &str = "hive.graphql.operation.hash";
pub const HIVE_GRAPHQL_SUBGRAPH_NAME: &str = "hive.graphql.subgraph.name";
/// Hive-specific attributes for errors
pub const HIVE_ERROR_AFFECTED_PATH: &str = "hive.error.affected_path";
pub const HIVE_ERROR_SUBGRAPH_NAME: &str = "hive.error.subgraph_name";
pub const HIVE_ERROR_PATH: &str = "hive.error.path";

/// Internal Hive attributes
pub const HIVE_INFLIGHT_ROLE: &str = "hive.inflight.role";
pub const HIVE_INFLIGHT_KEY: &str = "hive.inflight.key";
pub const HIVE_INFLIGHT_LINK_RELATIONSHIP: &str = "hive.inflight.relationship";

/// Generic/Common attributes
pub const CACHE_HIT: &str = "cache.hit";

/// Deprecated attributes (for backward compatibility with observability tools)
pub const DEPRECATED_HTTP_METHOD: &str = "http.method";
pub const DEPRECATED_HTTP_URL: &str = "http.url";
pub const DEPRECATED_HTTP_HOST: &str = "http.host";
pub const DEPRECATED_HTTP_SCHEME: &str = "http.scheme";
pub const DEPRECATED_HTTP_FLAVOR: &str = "http.flavor";
pub const DEPRECATED_HTTP_REQUEST_CONTENT_LENGTH: &str = "http.request_content_length";
pub const DEPRECATED_HTTP_USER_AGENT: &str = "http.user_agent";
pub const DEPRECATED_HTTP_TARGET: &str = "http.target";
pub const DEPRECATED_HTTP_STATUS_CODE: &str = "http.status_code";
pub const DEPRECATED_HTTP_RESPONSE_CONTENT_LENGTH: &str = "http.response_content_length";
pub const DEPRECATED_NET_PEER_NAME: &str = "net.peer.name";
pub const DEPRECATED_NET_PEER_PORT: &str = "net.peer.port";
pub const DEPRECATED_GRAPHQL_DOCUMENT: &str = "graphql.document";
