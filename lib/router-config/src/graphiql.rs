use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct GraphiQLConfig {
    /// Enables/disables the GraphiQL interface. By default, the GraphiQL interface is enabled.
    ///
    /// You can override this setting by setting the `GRAPHIQL_ENABLED` environment variable to `true` or `false`.
    #[serde(default = "default_graphiql_enabled")]
    pub enabled: bool,
}

fn default_graphiql_enabled() -> bool {
    true
}

impl Default for GraphiQLConfig {
    fn default() -> Self {
        Self {
            enabled: default_graphiql_enabled(),
        }
    }
}
