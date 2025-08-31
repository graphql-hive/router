use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, JsonSchema, Default)]
pub struct QueryPlannerConfig {
    /// A flag to allow exposing the query plan in the response.
    /// When set to `true` and an incoming request has a `hive-expose-query-plan: true` header, the query plan will be exposed in the response, as part of `extensions`.
    #[serde(default)]
    pub allow_expose: bool,
}
