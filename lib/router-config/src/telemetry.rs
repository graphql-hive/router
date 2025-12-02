use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::telemetry::tracing::TracingConfig;

pub mod tracing;

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct TelemetryConfig {
    #[serde(default)]
    pub tracing: TracingConfig,
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            tracing: TracingConfig::default(),
        }
    }
}
