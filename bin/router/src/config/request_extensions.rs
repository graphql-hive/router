use std::collections::HashMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Configuration for propagating `extensions` from the client request to the
/// subgraph requests.
#[derive(Debug, Default, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct RequestExtensionsConfig {
    /// Rules for propagating client request `extensions` to subgraphs.
    /// When omitted, client request `extensions` are not sent to subgraphs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub propagate: Option<RequestExtensionsPropagateConfig>,
}

/// Configuration for propagating client request extensions to subgraph requests.
///
/// ## Scopes
/// - The rule under `all` applies to every subgraph.
/// - The rule under `subgraphs.<name>` **replaces** the `all` rule for that subgraph.
///   A subgraph set to `null` receives no client request `extensions`.
///
/// When no rule applies to a subgraph, client request `extensions` are not sent to it.
///
/// The propagated extensions are sent with every subgraph request of the operation,
/// including entity fetches and subscriptions.
///
/// ## Router-owned keys
/// Some top-level keys are owned by the router and are never propagated, by any rule:
/// - `persistedQuery`, `headers` and `subscription`.
/// - The key set by `jwt.forward_claims_to_upstream_extensions.field_name`, when JWT claims forwarding is enabled.
/// - The first key of every `extensions.*` JSON path selector of `persisted_documents`, when persisted documents are enabled.
#[derive(Debug, Default, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct RequestExtensionsPropagateConfig {
    /// The rule applied to all subgraphs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub all: Option<RequestExtensionsPropagateRule>,

    /// Rules applied to individual subgraphs, replacing the `all` rule.
    /// Keys are subgraph names as defined in the supergraph schema.
    /// A subgraph set to `null` receives no client request `extensions`.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub subgraphs: HashMap<String, Option<RequestExtensionsPropagateRule>>,
}

/// Which top-level keys of the client request `extensions` are propagated.
#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct RequestExtensionsPropagateRule {
    /// How `keys` is applied to the top-level keys of the client request `extensions`.
    pub mode: RequestExtensionsPropagateMode,

    /// Top-level extension keys to allow or deny, depending on `mode`.
    pub keys: Vec<String>,
}

/// How `keys` is applied to the top-level keys of the client request `extensions`.
#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RequestExtensionsPropagateMode {
    /// Propagate only the listed keys.
    /// The list can't be empty, and can't include a key owned by the router.
    Allow,
    /// Propagate every key except the listed ones.
    /// An empty list propagates every key that isn't owned by the router.
    ///
    /// Note that any key a client sends is forwarded to the subgraphs of this rule,
    /// so prefer `allow` when subgraphs shouldn't receive arbitrary client data.
    Deny,
}
