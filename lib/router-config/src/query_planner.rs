use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct QueryPlannerConfig {
    /// A flag to allow exposing the query plan in the response.
    /// When set to `true` and an incoming request has a `hive-expose-query-plan: true` header, the query plan will be exposed in the response, as part of `extensions`.
    #[serde(default = "default_query_planning_allow_expose")]
    pub allow_expose: bool,
    /// The maximum time for the query planner to create an execution plan.
    /// This acts as a safeguard against overly complex or malicious queries that could degrade server performance.
    /// When the timeout is reached, the planning process is cancelled.
    ///
    /// Default: 10s.
    #[serde(
        default = "default_query_planning_timeout",
        deserialize_with = "humantime_serde::deserialize",
        serialize_with = "humantime_serde::serialize"
    )]
    #[schemars(with = "String")]
    pub timeout: Duration,
    /// Enables an experimental feature that folds matching object-type inline fragments
    /// into an interface fragment, even when that interface is not the field's declared return type.
    ///
    /// The fold is only applied when the concrete object branches select the same fields and
    /// exactly match the interface members in the target subgraph.
    ///
    /// Default: false.
    #[serde(default = "default_experimental_abstract_type_folding")]
    pub experimental_abstract_type_folding: bool,
}

impl Default for QueryPlannerConfig {
    fn default() -> Self {
        Self {
            allow_expose: default_query_planning_allow_expose(),
            timeout: default_query_planning_timeout(),
            experimental_abstract_type_folding: default_experimental_abstract_type_folding(),
        }
    }
}

fn default_query_planning_allow_expose() -> bool {
    false
}

fn default_query_planning_timeout() -> Duration {
    Duration::from_secs(10)
}

fn default_experimental_abstract_type_folding() -> bool {
    false
}
