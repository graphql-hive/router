use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct LaboratoryConfig {
    /// Enables/disables the Hive Laboratory interface. By default, the Hive Laboratory interface is enabled.
    ///
    /// You can override this setting by setting the `LABORATORY_ENABLED` environment variable to `true` or `false`.
    #[serde(default = "default_laboratory_enabled")]
    pub enabled: bool,
}

fn default_laboratory_enabled() -> bool {
    true
}

impl Default for LaboratoryConfig {
    fn default() -> Self {
        Self {
            enabled: default_laboratory_enabled(),
        }
    }
}
