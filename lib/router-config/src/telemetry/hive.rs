use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    primitives::value_or_expression::ValueOrExpression,
    telemetry::tracing::{BatchProcessorConfig, OtlpGrpcConfig, OtlpHttpConfig, OtlpProtocol},
};

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
#[derive(Default)]
pub struct HiveConfig {
    #[serde(default)]
    pub endpoint: ValueOrExpression<String>,
    #[serde(default)]
    pub token: ValueOrExpression<String>,
    #[serde(default)]
    pub target: ValueOrExpression<String>,
    #[serde(default)]
    pub tracing: TracingConfig,
}

impl Default for TracingConfig {
    fn default() -> Self {
        Self {
            enabled: default_tracing_enabled(),
            batch_processor: BatchProcessorConfig::default(),
            protocol: OtlpProtocol::Grpc,
            http: None,
            grpc: None,
        }
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct TracingConfig {
    #[serde(default = "default_tracing_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub batch_processor: BatchProcessorConfig,
    pub protocol: OtlpProtocol,
    #[serde(default)]
    pub http: Option<OtlpHttpConfig>,
    #[serde(default)]
    pub grpc: Option<OtlpGrpcConfig>,
}

fn default_tracing_enabled() -> bool {
    true
}
