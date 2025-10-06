use std::collections::HashMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Deserialize, Serialize, JsonSchema, Clone)]
pub struct OverrideSubgraphUrlsConfig {
    #[serde(default)]
    pub subgraphs: HashMap<String, OverrideSubgraphUrlConfig>
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
pub enum OverrideSubgraphUrlConfig {
    /// A static URL to override the subgraph's URL.
    Url(String),
    /// A dynamic URL that can be resolved at runtime.
    Expression(String),
}