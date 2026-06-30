use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Configuration for propagating `extensions` from subgraph responses to the
/// client response.
#[derive(Debug, Default, Deserialize, Serialize, JsonSchema, Clone)]
pub struct ExtensionsConfig {
    /// Rules for propagating subgraph response `extensions` to the client.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub propagate: Option<ExtensionsPropagateConfig>,
}

/// Configuration for propagating subgraph extensions to the client response.
#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
pub struct ExtensionsPropagateConfig {
    /// How to merge an extension key seen across multiple subgraph responses.
    /// Default: `last`.
    #[serde(default)]
    pub algorithm: ExtensionsMergeAlgo,

    /// Top-level extension keys allowed to propagate. When omitted, all keys
    /// are propagated. Any key not in this list is ignored.
    ///
    /// NOTE: `queryPlan` is a reserved key used by the router itself and will
    /// never be propagated from subgraphs regardless of this list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow: Option<Vec<String>>,
}

/// How to merge an extension key seen across multiple subgraph responses.
#[derive(Debug, Default, Deserialize, Serialize, JsonSchema, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionsMergeAlgo {
    /// Keep the first value encountered for a key, ignore later ones.
    /// Note that the subgraph response order is not guaranteed, so this may be non-deterministic.
    First,
    /// Overwrite with the last value encountered for a key.
    /// Note that the subgraph response order is not guaranteed, so this may be non-deterministic.
    /// Default.
    #[default]
    Last,
    /// Collect every value for a key into an array.
    ///
    /// When used, the resulting value for a key will always be an array,
    /// even if only one value was seen.
    Append,
}
