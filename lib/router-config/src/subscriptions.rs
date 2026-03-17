use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::{HashMap, HashSet};
use std::time::Duration;
use url::Url;

#[derive(Debug, Deserialize, Serialize, JsonSchema, Default)]
#[serde(deny_unknown_fields)]
pub struct SubscriptionsConfig {
    /// Enables/disables subscriptions. By default, the subscriptions are disabled.
    ///
    /// You can override this setting by setting the `SUBSCRIPTIONS_ENABLED` environment variable to `true` or `false`.
    #[serde(default)]
    pub enabled: bool,
    /// Configuration for subgraphs using the HTTP Callback protocol.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub callback: Option<CallbackConfig>,
    /// Configuration for subgraphs using WebSocket protocol.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub websocket: Option<WebSocketConfig>,
}

/// Configuration for the HTTP Callback subscription mode.
#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct CallbackConfig {
    /// The public URL that subgraphs will use to send callback messages to this router.
    ///
    /// Your public_url must match the server address combined with the router's path.
    /// Meaning, if your server is `http://localhost:4000` and the path is `/callback`,
    /// your `public_url` should be `http://localhost:4000/callback`.
    ///
    /// Example: `https://example.com:4000/callback`
    pub public_url: Url,
    /// The path of the router's callback endpoint.
    /// Must be an absolute path starting with `/`. Defaults to `/callback`.
    #[serde(
        default = "default_callback_path",
        deserialize_with = "deserialize_absolute_path"
    )]
    pub path: String,
    /// The interval at which the subgraph must send heartbeat messages.
    /// If set to 0, heartbeats are disabled. Defaults to 5 seconds.
    #[serde(
        default = "default_heartbeat_interval",
        deserialize_with = "humantime_serde::deserialize",
        serialize_with = "humantime_serde::serialize"
    )]
    #[schemars(with = "String")]
    pub heartbeat_interval: Duration,
    /// The list of subgraph names that use the HTTP callback protocol.
    #[serde(default)]
    pub subgraphs: HashSet<String>,
}

fn default_callback_path() -> String {
    "/callback".to_string()
}

fn deserialize_absolute_path<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let path = String::deserialize(deserializer)?;
    if !path.starts_with('/') {
        return Err(serde::de::Error::custom(format!(
            "callback path must be absolute (start with /), got: {path:?}"
        )));
    }
    Ok(path)
}

fn default_heartbeat_interval() -> Duration {
    Duration::from_secs(5)
}

/// Configuration for the WebSocket subscription mode.
#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct WebSocketConfig {
    /// The default configuration that will be applied to all subgraphs using
    /// WebSocket protocol, unless overridden by a specific subgraph configuration.
    ///
    /// When specified, all subgraphs (not claimed by `callback`) will use the WebSocket protocol.
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
        if let Some(ref callback) = self.callback {
            if callback.subgraphs.contains(subgraph_name) {
                return SubscriptionProtocol::HTTPCallback;
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callback_path_must_be_absolute() {
        let err = serde_json::from_str::<CallbackConfig>(
            r#"{"public_url": "http://localhost:4000/callback", "path": "callback"}"#,
        )
        .unwrap_err();
        assert!(
            err.to_string()
                .contains("callback path must be absolute (start with /)"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn callback_path_absolute_is_accepted() {
        let config = serde_json::from_str::<CallbackConfig>(
            r#"{"public_url": "http://localhost:4000/callback", "path": "/callback"}"#,
        )
        .unwrap();
        assert_eq!(config.path, "/callback");
    }

    #[test]
    fn callback_path_defaults_to_absolute() {
        let config = serde_json::from_str::<CallbackConfig>(
            r#"{"public_url": "http://localhost:4000/callback"}"#,
        )
        .unwrap();
        assert_eq!(config.path, "/callback");
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
    /// Uses the HTTP Callback protocol for subscriptions.
    /// See: https://www.apollographql.com/docs/graphos/routing/operations/subscriptions/callback-protocol
    HTTPCallback,
}
