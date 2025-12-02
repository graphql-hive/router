use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::telemetry::tracing::TracingConfig;

pub mod tracing;

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct TelemetryConfig {
    #[serde(default)]
    pub tracing: TracingConfig,
    #[serde(default)]
    pub service: ServiceConfig,
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            tracing: TracingConfig::default(),
            service: ServiceConfig::default(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct ServiceConfig {
    #[serde(default = "default_service_name")]
    pub name: String,
}

impl Default for ServiceConfig {
    fn default() -> Self {
        Self {
            name: default_service_name(),
        }
    }
}

fn default_service_name() -> String {
    "hive-router".to_string()
}
