use std::collections::HashMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Default, Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct DemandControlConfig {
    /// Setting this `true` to measure operation costs, or enforce the cost limits for the operation
    pub enabled: bool,

    /// If you want to enforce the cost limit, set the maximum allowed cost.
    /// If the cost of an operation exceeds this limit, the router will reject the request
    pub max_cost: Option<u64>,

    /// The assumed maximum size of a list for fields that return lists.
    pub list_size: Option<usize>,

    /// Subgraph-level demand control configuration.
    /// In addition to the existing global cost limit for the whole supergraph.
    /// This helps you to protect individual subgraphs from expensive operations,
    /// and to get more fine-grained control over the costs of operations.
    ///
    /// When a subgraph-specific cost limit is exceeded,
    /// - The router will continue running the rest of the query plan, including other subgraphs within the limits
    /// - Skips calls to the specific subgraph that exceeded the cost limit, and returns an error for that subgraph
    pub subgraph: Option<SubgraphLevelDemandControlConfig>,

    /// Whether to include the calculated cost in the response extensions.
    pub include_extension_metadata: Option<bool>,

    /// Optional actual cost calculation configuration.
    ///
    /// When enabled, the response extension metadata will include
    /// `cost.actual` and `cost.delta` in addition to the estimated cost.
    pub actual_cost: Option<DemandControlActualCostConfig>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct DemandControlActualCostConfig {
    pub mode: DemandControlActualCostMode,
}

#[derive(Default, Debug, Deserialize, Serialize, JsonSchema, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum DemandControlActualCostMode {
    BySubgraph,
    #[default]
    ByResponseShape,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct SubgraphLevelDemandControlConfig {
    pub all: Option<DemandControlSubgraphConfig>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub subgraphs: HashMap<String, DemandControlSubgraphConfig>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(rename_all = "lowercase")]
pub struct DemandControlSubgraphConfig {
    /// If you want to enforce the cost limit, set the maximum allowed cost.
    /// If the cost of an operation exceeds this limit, the router will reject the request
    pub max_cost: Option<u64>,

    /// The assumed maximum size of a list for fields that return lists.
    pub list_size: Option<usize>,
}
