use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
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
    /// Can also be set via the `QUERY_PLANNER_EXPERIMENTAL_ABSTRACT_TYPE_FOLDING` environment variable.
    ///
    /// Default: false.
    #[serde(default = "default_experimental_abstract_type_folding")]
    pub experimental_abstract_type_folding: bool,
    /// Rewrites hard-coded field argument values into variables, so operations that differ
    /// only in those values share one query plan.
    /// For example, `{ user(id: "1") { name } }` is planned as `query($_v0: ID!) { user(id: $_v0) { name } }`.
    ///
    /// Without it, every distinct value makes a new plan cache entry,
    /// and a client looping over IDs can push every other plan out of the cache.
    ///
    /// Subgraphs receive the values as variables instead of inline. A subgraph parses a custom
    /// scalar given as a variable with its `parseValue`, not `parseLiteral`, so turn this off if
    /// a subgraph has a scalar where the two behave differently.
    ///
    /// Default: true.
    #[serde(default = "default_arguments_to_variables")]
    pub arguments_to_variables: bool,
}

impl Default for QueryPlannerConfig {
    fn default() -> Self {
        Self {
            allow_expose: default_query_planning_allow_expose(),
            timeout: default_query_planning_timeout(),
            experimental_abstract_type_folding: default_experimental_abstract_type_folding(),
            arguments_to_variables: default_arguments_to_variables(),
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

fn default_arguments_to_variables() -> bool {
    true
}
