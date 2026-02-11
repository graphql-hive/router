use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Deserialize, Serialize, JsonSchema, Default)]
#[serde(deny_unknown_fields)]
pub struct SubscriptionsConfig {
    /// Enables/disables subscriptions. By default, the subscriptions are disabled.
    ///
    /// You can override this setting by setting the `SUBSCRIPTIONS_ENABLED` environment variable to `true` or `false`.
    #[serde(default)]
    pub enabled: bool,
    /// Configuration for subgraphs using WebSocket protocol.
    #[serde(default)]
    pub websocket: Option<WebSocketConfig>,
}

/// Configuration for the WebSocket subscription mode.
#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct WebSocketConfig {
    /// The default configuration that will be applied to all subgraphs using
    /// WebSocket protocol, unless overridden by a specific subgraph configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub all: Option<WebSocketSubgraphConfig>,
    /// Optional per-subgraph configurations that will override the default configuration for specific subgraphs.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub subgraphs: HashMap<String, WebSocketSubgraphConfig>,
}

/// WebSocket configuration for a specific subgraph or the default for all subgraphs.
#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct WebSocketSubgraphConfig {
    /// Determines the URL path to use for the subscription endpoint:
    ///
    /// - For WebSocket connections, the URL will be `ws://<subgraph-url><path>`.
    /// - If `path` is not set, the default subgraph URL is used, with the scheme adjusted to `ws`
    ///   for WebSocket connections where applicable.
    ///
    /// Note to always provide the absolute path starting with a `/`, e.g., `/ws`.
    ///
    /// For example, if the subgraph URL is `http://example.com/graphql` and the path is set to `/ws`,
    /// the resulting WebSocket URL will be `ws://example.com/ws`.
    #[serde(default)]
    pub path: Option<String>,
}

impl SubscriptionsConfig {
    /// Returns the subscription protocol for the given subgraph.
    /// Returns HTTP (streaming) as the default if no specific mode is configured.
    pub fn get_protocol_for_subgraph(&self, subgraph_name: &str) -> SubscriptionProtocol {
        if let Some(ref websocket) = self.websocket {
            if websocket.all.is_some() || websocket.subgraphs.contains_key(subgraph_name) {
                return SubscriptionProtocol::WebSocket;
            }
        }
        SubscriptionProtocol::HTTP
    }

    /// Returns the WebSocket path for the given subgraph, if configured.
    /// Checks the subgraph-specific configuration first, then falls back to the `all` default.
    pub fn get_websocket_path(&self, subgraph_name: &str) -> Option<&str> {
        self.websocket.as_ref().and_then(|ws| {
            ws.subgraphs
                .get(subgraph_name)
                .and_then(|s| s.path.as_deref())
                .or_else(|| ws.all.as_ref().and_then(|a| a.path.as_deref()))
        })
    }
}

/// The selected protocol for the subscriptions towards subgraphs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SubscriptionProtocol {
    /// Uses any HTTP streaming protocol that the subgraph accepts. Supported protocols are:
    /// - Server-Sent Events (SSE). Respecting only the "distinct connection mode" of the GraphQL over SSE specification. See: https://github.com/graphql/graphql-over-http/blob/main/rfcs/GraphQLOverSSE.md#distinct-connections-mode.
    /// - Apollo Multipart HTTP. Implements the Apollo's Multipart HTTP specification. See: https://www.apollographql.com/docs/graphos/routing/operations/subscriptions/multipart-protocol.
    /// - GraphQL Incremental Delivery. Implements the official GraphQL Incremental Delivery specification. See: https://github.com/graphql/graphql-over-http/blob/main/rfcs/IncrementalDelivery.md.
    #[default]
    HTTP,
    /// Uses GraphQL over WebSocket (graphql-transport-ws subprotocol).
    WebSocket,
}
