use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Deserialize, Serialize, JsonSchema, Default)]
#[serde(deny_unknown_fields)]
pub struct SubscriptionsConfig {
    /// Enables/disables subscriptions. By default, the subscriptions are disabled.
    ///
    /// You can override this setting by setting the `SUBSCRIPTIONS_ENABLED` environment variable to `true` or `false`.
    #[serde(default = "default_subscriptions_enabled")]
    pub enabled: bool,
    /// The default configuration that will be applied to all subgraphs, unless overridden by a specific subgraph configuration.
    #[serde(default)]
    pub all: SubscriptionsSubgraphConfig,
    /// Optional per-subgraph configurations that will override the default configuration for specific subgraphs.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub subgraphs: HashMap<String, SubscriptionsSubgraphConfig>,
}

fn default_subscriptions_enabled() -> bool {
    false
}

/// The selected protocol for the subscriptions towards subgraphs.
#[derive(Debug, Deserialize, Serialize, JsonSchema, Default)]
#[serde(rename_all = "lowercase")]
pub enum SubscriptionProtocol {
    /// Uses any HTTP streaming protocol that the subgraph accepts. Supported protocols are:
    /// - Server-Sent Events (SSE). Respecting only the "distinct connection mode" of the GraphQL over SSE specification. See: https://github.com/graphql/graphql-over-http/blob/main/rfcs/GraphQLOverSSE.md#distinct-connections-mode.
    /// - Apollo Multipart HTTP. Implements the Apollo's Multipart HTTP specification. See: https://www.apollographql.com/docs/graphos/routing/operations/subscriptions/multipart-protocol.
    /// - GraphQL Incremental Delivery. Implements the official GraphQL Incremental Delivery specification. See: https://github.com/graphql/graphql-over-http/blob/main/rfcs/IncrementalDelivery.md.
    #[default]
    HTTP,
    /// Uses GraphQL over WebSocket (graphql-transport-ws subprotocol) to establish
    /// subscriptions towards subgraphs.
    WebSocket,
}

/// Configuration for subscription connections to subgraphs.
#[derive(Debug, Deserialize, Serialize, JsonSchema, Default)]
#[serde(deny_unknown_fields)]
pub struct SubscriptionsSubgraphConfig {
    /// The selected protocol for the subscriptions towards the subgraph(s).
    #[serde(default)]
    pub protocol: SubscriptionProtocol,
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
    pub path: Option<String>,
}
