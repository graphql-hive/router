use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, JsonSchema, Default)]
#[serde(deny_unknown_fields)]
pub struct WebSocketConfig {
    /// Enables/disables WebSocket connections. By default, WebSockets are disabled.
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

    /// Whether to accept headers in the connection init payload of WebSocket connections.
    ///
    /// Defaults to `true`.
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
    #[serde(default = "default_headers_in_connection_init_payload")]
    pub headers_in_connection_init_payload: bool,

    /// Whether to accept headers in the `extensions` field of the GraphQL operation inside WebSocket connections.
    ///
    /// For example, if the client sends a GraphQL operation like:
    ///
    /// ```json
    /// {
    ///   "query": "subscription { greetings }",
    ///   "extensions": {
    ///     "headers": {
    ///       "Authorization": "Bearer abc123"
    ///     }
    ///   }
    /// }
    /// ```
    ///
    /// The headers will be extracted and considered for the WebSocket connection to the subgraph
    /// respecting the header propagation rules as well as validating against any
    /// JWT rules defined in the configuration.
    ///
    /// Note that the connection init message payload can also contain headers, and those will be
    /// considered as well. If the same header is defined both in the connection init message
    /// and in the operation extensions, the value from the operation extensions will take precedence
    /// if this option is enabled.
    #[serde(default)]
    pub headers_in_operation_extensions: bool,

    /// Whether to merge headers from both connection init payload and operation extensions for WebSocket
    /// connections. Meaning, headers from both sources will be combined, with operation extensions
    /// taking precedence in case of conflicts.
    ///
    /// Defaults to `false`.
    ///
    /// Needs both `headers_in_connection_init_payload` and `headers_in_operation_extensions`
    /// to be enabled to have any effect.
    ///
    /// This is useful when dealing with authentication using tokens that expire, where the
    /// initial connection might use one token, but subsequent operations might need to
    /// provide updated tokens in the operation extensions and then use that for further authentication.
    #[serde(default)]
    pub merge_connection_init_payload_and_operation_extensions_headers: bool,
}

fn default_headers_in_connection_init_payload() -> bool {
    true
}
