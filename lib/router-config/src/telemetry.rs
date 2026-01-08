use std::collections::HashMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::primitives::value_or_expression::ValueOrExpression;
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
    pub resource: ResourceConfig,
}

impl TelemetryConfig {
    pub fn is_tracing_enabled(&self) -> bool {
        self.tracing.is_enabled() || self.hive.as_ref().is_some_and(|hive| hive.tracing.enabled)
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct ResourceConfig {
    #[serde(default)]
    pub attributes: HashMap<String, ValueOrExpression<String>>,
}
