use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
pub struct QueryPlannerConfig {
    /// A flag to allow exposing the query plan in the response.
    /// When set to `true` and an incoming request has a `hive-expose-query-plan: true` header, the query plan will be exposed in the response, as part of `extensions`.
    #[serde(default = "default_query_planning_allow_expose")]
    pub allow_expose: bool,
    /// The maximum time in milliseconds for the query planner to create an execution plan.
    /// This acts as a safeguard against overly complex or malicious queries that could degrade server performance.
    /// When the timeout is reached, the planning process is cancelled.
    ///
    /// Default: 10000 (10 seconds).
    #[serde(default = "default_query_planning_timeout_ms")]
    pub timeout_ms: u64,
}

impl Default for QueryPlannerConfig {
    fn default() -> Self {
        Self {
            allow_expose: default_query_planning_allow_expose(),
            timeout_ms: default_query_planning_timeout_ms(),
        }
    }
}

fn default_query_planning_allow_expose() -> bool {
    false
}

fn default_query_planning_timeout_ms() -> u64 {
    10_000
}
