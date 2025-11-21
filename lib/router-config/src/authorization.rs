use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct AuthorizationConfig {
    pub directives: AuthorizationDirectivesConfig,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct AuthorizationDirectivesConfig {
    #[serde(default = "default_directives_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub unauthorized: UnauthorizedConfig,
}

#[derive(Default, Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct UnauthorizedConfig {
    #[serde(default)]
    pub mode: UnauthorizedMode,
}

#[derive(Debug, Default, Deserialize, Serialize, JsonSchema, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum UnauthorizedMode {
    #[default]
    Filter,
    Reject,
}

fn default_directives_enabled() -> bool {
    true
}

impl Default for AuthorizationDirectivesConfig {
    fn default() -> Self {
        Self {
            enabled: default_directives_enabled(),
            unauthorized: Default::default(),
        }
    }
}
