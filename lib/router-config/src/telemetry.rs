use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::telemetry::{hive::HiveTelemetryConfig, tracing::TracingConfig};

pub mod hive;
pub mod tracing;

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
#[derive(Default)]
pub struct TelemetryConfig {
    #[serde(default)]
    pub hive: Option<HiveTelemetryConfig>,
    #[serde(default)]
    pub tracing: TracingConfig,
    #[serde(default)]
    pub service: ServiceConfig,
}

impl TelemetryConfig {
    pub fn is_tracing_enabled(&self) -> bool {
        self.tracing.is_enabled() || self.hive.as_ref().is_some_and(|hive| hive.tracing.enabled)
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
