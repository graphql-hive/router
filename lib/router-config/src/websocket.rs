use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, JsonSchema, Default)]
#[serde(deny_unknown_fields)]
pub struct WebSocketConfig {
    /// Enables/disables WebSocket connections.
    ///
    /// By default, WebSockets are disabled.
    ///
    /// You can override this setting by setting the `WEBSOCKET_ENABLED` environment variable to `true` or `false`.
    #[serde(default)]
    pub enabled: bool,

    /// The path to use for the WebSocket endpoint on the router.
    ///
    /// Note to always provide the absolute path starting with a `/`, e.g., `/ws`.
    ///
    /// By default, the WebSocket endpoint will be available at the `http.graphql_endpoint` (defaults to `/graphql`)
    /// if no path is specified and the clients will connect using `ws://<router-url>/<graphql_endpoint>`.
    #[serde(default)]
    pub path: Option<String>,

    /// Configuration for handling headers for WebSocket connections.
    #[serde(default)]
    pub headers: WebSocketHeadersConfig,
}

#[derive(Default, Deserialize, Serialize, JsonSchema, Debug)]
#[serde(rename_all = "lowercase")]
pub enum WebSocketHeadersSource {
    /// Do not accept headers from any source inside WebSocket connections.
    None,
    /// Accept headers from the connection init payload. This is the default.
    ///
    /// For example, if the client sends a connection init message like:
    ///
    /// ```json
    /// {
    ///   "type": "connection_init",
    ///   "payload": {
    ///     "Authorization": "Bearer abc123"
    ///   }
    /// }
    /// ```
    ///
    /// The headers will be extracted and considered for the WebSocket connection to the subgraph
    /// respecting the header propagation rules as well as validating against any
    /// JWT rules defined in the configuration.
    ///
    /// Note that there is no `headers` field in the payload, so all fields in the payload
    /// will be treated as headers when this option is enabled.
    #[default]
    Connection,
    /// Accept headers from the `headers` field from the GraphQL operation extensions.
    ///
    /// For example, if the connected client sends a GraphQL operation like:
    ///
    /// ```json
    /// {
    ///   "query": "{ topProducts { name } }",
    ///   "extensions": {
    ///     "headers": {
    ///       "Authorization": "Bearer abc123"
    ///     }
    ///   }
    /// }
    /// ```
    ///
    /// The headers will be extracted and considered for the subgraph respecting the header
    /// propagation rules as well as validating against any JWT rules defined in the configuration.
    Operation,
    /// Accept headers from both the connection init payload and the operation extensions.
    ///
    /// Headers from the operation extensions will take precedence over those from the connection init
    /// payload when both are provided.
    Both,
}

#[derive(Default, Deserialize, Serialize, JsonSchema, Debug)]
#[serde(deny_unknown_fields)]
pub struct WebSocketHeadersConfig {
    /// The source(s) from which to accept headers for WebSocket connections.
    pub source: WebSocketHeadersSource,
    /// Whether to persist merged headers for the duration of the WebSocket connection
    /// when using the `both` source (headers are accepted from multiple sources).
    ///
    /// Only has effect when `source` is set to `both`.
    ///
    /// This is useful when dealing with authentication using tokens that expire, where the
    /// initial connection might use one token, but subsequent operations might need to
    /// provide updated tokens in the operation extensions and then use that for further authentication.
    ///
    /// For example:
    ///
    /// 1. Client connects with connection init payload containing an Authorization header with a token.
    /// 2. Client sends a subscription operation with an updated Authorization header in the operation extensions.
    /// 3. If `persist` is enabled, the updated Authorization header will be stored and used for subsequent operations.
    #[serde(default)]
    pub persist: bool,
}

impl WebSocketHeadersConfig {
    pub fn accepts_connection_headers(&self) -> bool {
        matches!(
            self.source,
            WebSocketHeadersSource::Connection | WebSocketHeadersSource::Both
        )
    }
    pub fn accepts_operation_headers(&self) -> bool {
        matches!(
            self.source,
            WebSocketHeadersSource::Operation | WebSocketHeadersSource::Both
        )
    }
}
