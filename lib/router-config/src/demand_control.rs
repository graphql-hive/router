use std::collections::HashMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::primitives::http_header::HttpHeaderName;

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct DemandControlConfig {
    /// Enable demand control processing. Must be `true` for any cost estimation,
    /// enforcement or telemetry to take effect.
    pub enabled: bool,

    /// The headers to expose in the response.
    /// Headers are exposed in the response, in both cases when the request is rejected or when it is allowed to proceed.
    ///
    /// Defaults to none.
    #[serde(default = "DemandControlExposeHeadersConfig::default")]
    pub expose_headers: DemandControlExposeHeadersConfig,

    /// Configuration for operation cost limits.
    ///
    /// This controls the maximum cost allowed for a single operation executed against the Router, based on the estimated value.
    /// When the estimated cost exceeds this value, the request is rejected before any subgraph is contacted.
    pub operation_cost: OperationCostConfig,

    /// The default list size to use when `@listSize` is not specified in the schema.
    #[serde(default = "DefaultListSizeConfig::default")]
    pub default_list_size: DefaultListSizeConfig,

    /// Subgraph cost limit configuration, including the mode to use for subgraph budget enforcement.
    pub subgraphs_budget: SubgraphsBudgetConfig,

    /// How actual cost is computed after execution.
    ///
    /// - `by_subgraph` (default): sum the cost computed per individual subgraph
    ///   fetch responses.
    /// - `by_response_shape`: walk the merged supergraph response and reapply
    ///   the static cost rules. Does not account for intermediate subgraph
    ///   work.
    ///
    /// Note: the "actual" value calculated in any mode is not used for enforcment.
    #[serde(default)]
    pub actual_cost_mode: DemandControlActualCostMode,
}

/// Default header name used to expose the configured `max` cost limit when
/// `max` is set to `true`.
const DEFAULT_MAX_HEADER_NAME: &str = "X-Cost-Max";
/// Default header name used to expose the estimated cost when `estimated` is
/// set to `true`.
const DEFAULT_ESTIMATED_HEADER_NAME: &str = "X-Cost-Estimated";
/// Default header name used to expose the actual cost when `actual` is set to
/// `true`.
const DEFAULT_ACTUAL_HEADER_NAME: &str = "X-Cost-Actual";

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, PartialEq, Eq)]
pub struct OperationCostConfig {
    /// The maximum cost allowed for a single operation, based on the estimated value.
    ///
    /// When the estimated cost exceeds this value, the request is rejected before any subgraph is contacted.
    pub max: u64,

    // Controls what happens when a cost estimation limit is exceeded.
    ///
    /// - `enforce`: reject the incoming request when a limit is breached.
    /// - `measure`: never reject. Cost is still computed, result codes are
    ///   recorded in telemetry (trace, logs, metrics), but no request is
    ///   blocked. Useful for shadowing a limit in production before switching
    ///   to `enforce`.
    pub mode: DemandControlMode,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
#[derive(Default)]
pub struct DemandControlExposeHeadersConfig {
    #[serde(default, deserialize_with = "deserialize_max_header")]
    pub max: Option<HttpHeaderName>,
    #[serde(default, deserialize_with = "deserialize_estimated_header")]
    pub estimated: Option<HttpHeaderName>,
    #[serde(default, deserialize_with = "deserialize_actual_header")]
    pub actual: Option<HttpHeaderName>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum HeaderNameOrBool {
    Bool(bool),
    Name(String),
}

/// Deserializes an expose-header field that may be set to a boolean or an
/// explicit header name:
///
/// - `true` -> `Some(default_header_name)`
/// - `false` -> `None`
/// - `"X-My-Header"` -> `Some("X-My-Header")`
///
fn deserialize_header_name_or_bool<'de, D>(
    deserializer: D,
    default_header_name: &'static str,
) -> Result<Option<HttpHeaderName>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    match HeaderNameOrBool::deserialize(deserializer)? {
        HeaderNameOrBool::Bool(true) => Ok(Some(
            HttpHeaderName::new(default_header_name).map_err(serde::de::Error::custom)?,
        )),
        HeaderNameOrBool::Bool(false) => Ok(None),
        HeaderNameOrBool::Name(name) => Ok(Some(
            HttpHeaderName::new(name).map_err(serde::de::Error::custom)?,
        )),
    }
}

fn deserialize_max_header<'de, D>(deserializer: D) -> Result<Option<HttpHeaderName>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    deserialize_header_name_or_bool(deserializer, DEFAULT_MAX_HEADER_NAME)
}

fn deserialize_estimated_header<'de, D>(deserializer: D) -> Result<Option<HttpHeaderName>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    deserialize_header_name_or_bool(deserializer, DEFAULT_ESTIMATED_HEADER_NAME)
}

fn deserialize_actual_header<'de, D>(deserializer: D) -> Result<Option<HttpHeaderName>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    deserialize_header_name_or_bool(deserializer, DEFAULT_ACTUAL_HEADER_NAME)
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DemandControlMode {
    Enforce,
    Measure,
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

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct SubgraphsBudgetConfig {
    /// The mode to use for subgraph budget enforcement.
    ///
    /// In `mode: enforce`, when a subgraph limit is exceeded:
    /// - The router **continues** executing the rest of the query plan.
    /// - The specific subgraph fetch is skipped and a `SUBGRAPH_COST_ESTIMATED_TOO_EXPENSIVE` error is added to the response.
    /// - The fetch call assumes error, and returns `null` as subgraph response.
    ///
    /// This kind of enforcement is applied to each subgraph fetch individually, during execution,
    /// in order to prevent false-positives from exceeding the limit.
    ///
    /// In `mode: measure`, subgraph limits are never enforced.
    pub mode: DemandControlMode,
    /// Default limit configuration applied to every subgraph unless overridden.
    pub all: Option<usize>,
    /// Per-subgraph overrides. Keys are subgraph names.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subgraphs: Option<HashMap<String, usize>>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
#[derive(Default)]
pub struct DefaultListSizeConfig {
    /// Default list size for fields in the supergraph that have no `@listSize` directive.
    pub all: Option<usize>,
    /// Per-subgraph overrides. Keys are subgraph names.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subgraphs: Option<HashMap<String, usize>>,
}
