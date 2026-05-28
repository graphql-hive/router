use std::collections::HashMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct DemandControlConfig {
    /// Enable demand control processing. Must be `true` for any cost estimation,
    /// enforcement or telemetry to take effect.
    pub enabled: bool,

    /// Controls what happens when a cost limit is exceeded.
    ///
    /// - `enforce`: reject the request (or skip the specific subgraph)
    ///   when a limit is breached. Requires `strategy.static_estimated.max`
    ///   and/or per-subgraph `max` to have any enforcement effect.
    /// - `measure`: never reject. Cost is still computed, result codes are
    ///   recorded in telemetry and in `extensions.cost`, but no request is
    ///   blocked. Useful for shadowing a limit in production before switching
    ///   to `enforce`.
    pub mode: DemandControlMode,

    /// When `true`, a `cost` object is appended to `extensions` on every
    /// response. It includes `estimated`, `result`, `bySubgraph`,
    /// `formulaCacheHit`, `estimatedFormulaBySubgraph`, `maxCost`, and
    /// (when `actual_cost_mode` is configured) `actual`, `delta`,
    /// `actualBySubgraph`.
    pub include_extension_metadata: Option<bool>,

    /// The cost estimation strategy. Currently only `static_estimated` is
    /// supported, which estimates cost before execution using `@cost` and
    /// `@listSize` directives.
    pub strategy: DemandControlStrategy,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DemandControlMode {
    Enforce,
    Measure,
}

/// Cost estimation strategy configuration.
#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub enum DemandControlStrategy {
    /// Statically estimates the cost of an operation before execution using
    /// the `@cost` and `@listSize` directives from the IBM Cost Specification.
    StaticEstimated(StaticEstimatedConfig),
}

impl DemandControlStrategy {
    pub fn static_estimated(&self) -> &StaticEstimatedConfig {
        let Self::StaticEstimated(cfg) = self;
        cfg
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct StaticEstimatedConfig {
    /// Supergraph-wide cost ceiling (in cost units).
    ///
    /// In `mode: enforce`, when the *estimated* cost of an operation exceeds
    /// this value, the request is rejected before any subgraph is contacted
    /// (`COST_ESTIMATED_TOO_EXPENSIVE`). When the *actual* cost (post-execution)
    /// exceeds this value, a `COST_ACTUAL_TOO_EXPENSIVE` error is appended to
    /// the response while `data` is still returned.
    ///
    /// In `mode: measure`, `max` still determines the `result` code in
    /// `extensions.cost` but does not cause rejection.
    pub max: u64,

    /// Default assumed list size for fields that have no `@listSize` directive.
    /// Per-subgraph overrides take precedence when configured.
    pub list_size: Option<usize>,

    /// How actual cost is computed after execution. Actual cost is always
    /// computed regardless of this setting; only the computation method differs.
    /// `extensions.cost` will always include `actual`, `delta` and (in
    /// `by_subgraph` mode) `actualBySubgraph`.
    ///
    /// - `by_subgraph` (default): sum the cost computed per individual subgraph
    ///   fetch response. Enables the `actualBySubgraph` extension field and more
    ///   closely mirrors the estimation strategy.
    /// - `by_response_shape`: walk the merged supergraph response and reapply
    ///   the static cost rules. Does not account for intermediate subgraph
    ///   work. This was the only option prior to `by_subgraph`.
    #[serde(default)]
    pub actual_cost_mode: DemandControlActualCostMode,

    /// Per-subgraph cost limits, in addition to the supergraph-wide `max`.
    ///
    /// `subgraph.all` provides defaults inherited by every subgraph;
    /// `subgraph.subgraphs.<name>` overrides them for a specific subgraph.
    ///
    /// In `mode: enforce`, when a subgraph limit is exceeded:
    /// - The router **continues** executing the rest of the query plan.
    /// - The specific subgraph fetch is skipped and a
    ///   `SUBGRAPH_COST_ESTIMATED_TOO_EXPENSIVE` error is returned for it.
    ///
    /// In `mode: measure`, subgraph limits are never enforced.
    #[serde(default)]
    pub subgraph: SubgraphLevelDemandControlConfig,
}

#[derive(Default, Debug, Deserialize, Serialize, JsonSchema, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DemandControlActualCostMode {
    /// Computes the cost of each subgraph response and sums them to get the
    /// total query cost. This is the default and preferred mode.
    #[default]
    BySubgraph,
    /// Computes the cost based on the final shape of the merged response.
    /// Does not account for intermediate subgraph work. This was the only
    /// option prior to `by_subgraph`.
    ByResponseShape,
}

#[derive(Default, Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct SubgraphLevelDemandControlConfig {
    /// Default limit configuration applied to every subgraph unless overridden.
    pub all: Option<DemandControlSubgraphConfig>,
    /// Per-subgraph overrides. Keys are subgraph names.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subgraphs: Option<HashMap<String, DemandControlSubgraphConfig>>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct DemandControlSubgraphConfig {
    /// Cost ceiling for this subgraph (in cost units). In `mode: enforce`,
    /// when the estimated cost of fetches to this subgraph exceeds this value,
    /// the subgraph is skipped and a `SUBGRAPH_COST_ESTIMATED_TOO_EXPENSIVE`
    /// error is returned. The rest of the query plan continues to run.
    /// Has no effect in `mode: measure`.
    pub max: Option<u64>,

    /// Default assumed list size for fields in this subgraph that have no
    /// `@listSize` directive. Overrides the global
    /// `strategy.static_estimated.list_size`.
    pub list_size: Option<usize>,
}
