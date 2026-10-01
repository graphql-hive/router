use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, Default)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ExecutionConfig {
    /// Enables an experimental executor that runs the query plan by the fetch-graph
    /// dependencies the planner recorded, instead of by the `Parallel` waves of the plan.
    ///
    /// The waves in the plan output are a presentation detail, and a wave boundary makes
    /// every fetch in it wait for the slowest one. With this enabled a fetch starts as
    /// soon as the fetches it actually depends on are merged, so a plan takes as long as
    /// its critical path. Plans using `@defer` still run wave by wave.
    ///
    /// Default: false.
    #[serde(default)]
    pub experimental_dependency_aware_execution: bool,
}
