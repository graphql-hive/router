use std::collections::HashMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::primitives::value_or_expression::ValueOrExpression;
use crate::telemetry::{hive::HiveTelemetryConfig, metrics::MetricsConfig, tracing::TracingConfig};

pub mod hive;
pub mod metrics;
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
    pub metrics: MetricsConfig,
    #[serde(default)]
    pub resource: ResourceConfig,
    #[serde(default)]
    pub client_identification: ClientIdentificationConfig,
}

impl TelemetryConfig {
    pub fn is_tracing_enabled(&self) -> bool {
        self.tracing.is_enabled() || self.hive.as_ref().is_some_and(|hive| hive.tracing.enabled)
    }

    pub fn is_metrics_enabled(&self) -> bool {
        self.metrics.is_enabled()
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct ResourceConfig {
    #[serde(default)]
    pub attributes: HashMap<String, ValueOrExpression<String>>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct ClientIdentificationConfig {
    #[serde(default = "default_client_name_header")]
    pub name_header: String,
    #[serde(default = "default_client_version_header")]
    pub version_header: String,
}

impl Default for ClientIdentificationConfig {
    fn default() -> Self {
        Self {
            name_header: default_client_name_header(),
            version_header: default_client_version_header(),
        }
    }
}

fn default_client_name_header() -> String {
    "graphql-client-name".to_string()
}

fn default_client_version_header() -> String {
    "graphql-client-version".to_string()
}
