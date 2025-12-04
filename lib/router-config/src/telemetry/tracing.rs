use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::primitives::value_or_expression::ValueOrExpression;

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct TracingConfig {
    #[serde(default)]
    pub collect: TracingCollectConfig,
    #[serde(default)]
    pub exporters: Vec<TracingExporterConfig>,
    #[serde(default)]
    pub propagation: TracingPropagationConfig,
}

impl Default for TracingConfig {
    fn default() -> Self {
        Self {
            collect: TracingCollectConfig::default(),
            exporters: Default::default(),
            propagation: TracingPropagationConfig::default(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct TracingCollectConfig {
    #[serde(default = "default_max_events_per_span")]
    pub max_events_per_span: u32,
    #[serde(default = "default_max_attributes_per_span")]
    pub max_attributes_per_span: u32,
    #[serde(default = "default_max_attributes_per_event")]
    pub max_attributes_per_event: u32,
    #[serde(default = "default_max_attributes_per_link")]
    pub max_attributes_per_link: u32,
    #[serde(default = "default_sampling")]
    pub sampling: f64,
    #[serde(default = "default_parent_based_sampler")]
    pub parent_based_sampler: bool,
}

fn default_max_events_per_span() -> u32 {
    128
}
fn default_max_attributes_per_span() -> u32 {
    128
}
fn default_max_attributes_per_event() -> u32 {
    16
}
fn default_max_attributes_per_link() -> u32 {
    32
}
fn default_sampling() -> f64 {
    1.0
}
fn default_parent_based_sampler() -> bool {
    false
}

impl Default for TracingCollectConfig {
    fn default() -> Self {
        Self {
            max_events_per_span: default_max_events_per_span(),
            max_attributes_per_span: default_max_attributes_per_span(),
            max_attributes_per_event: default_max_attributes_per_event(),
            max_attributes_per_link: default_max_attributes_per_link(),
            sampling: default_sampling(),
            parent_based_sampler: default_parent_based_sampler(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct TracingOtlpConfig {
    #[serde(default = "default_otlp_config_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub batch_processor: BatchProcessorConfig,
    #[serde(default)]
    pub endpoint: ValueOrExpression<String>,
    pub protocol: OtlpProtocol,
    #[serde(default)]
    pub http: Option<OtlpHttpConfig>,
    #[serde(default)]
    pub grpc: Option<OtlpGrpcConfig>,
}

fn default_otlp_config_enabled() -> bool {
    true
}

#[derive(Debug, Default, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct OtlpHttpConfig {
    #[serde(default)]
    pub headers: std::collections::HashMap<String, ValueOrExpression<String>>,
}

#[derive(Debug, Default, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct OtlpGrpcConfig {
    #[serde(default)]
    pub metadata: std::collections::HashMap<String, ValueOrExpression<String>>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields, tag = "kind")]
pub enum TracingExporterConfig {
    #[serde(rename = "otlp")]
    Otlp(TracingOtlpConfig),
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct BatchProcessorConfig {
    #[serde(default = "default_batch_max_concurrent_exports")]
    pub max_concurrent_exports: u32,
    #[serde(default = "default_batch_max_export_batch_size")]
    pub max_export_batch_size: u32,
    #[serde(default = "default_batch_max_queue_size")]
    pub max_queue_size: u32,
    #[serde(
        default = "default_batch_max_export_timeout",
        deserialize_with = "humantime_serde::deserialize",
        serialize_with = "humantime_serde::serialize"
    )]
    #[schemars(with = "String")]
    pub max_export_timeout: Duration,
    #[serde(
        default = "default_batch_scheduled_delay",
        deserialize_with = "humantime_serde::deserialize",
        serialize_with = "humantime_serde::serialize"
    )]
    #[schemars(with = "String")]
    pub scheduled_delay: Duration,
}

impl Default for BatchProcessorConfig {
    fn default() -> Self {
        Self {
            max_concurrent_exports: default_batch_max_concurrent_exports(),
            max_export_batch_size: default_batch_max_export_batch_size(),
            max_export_timeout: default_batch_max_export_timeout(),
            max_queue_size: default_batch_max_queue_size(),
            scheduled_delay: default_batch_scheduled_delay(),
        }
    }
}

fn default_batch_max_concurrent_exports() -> u32 {
    1
}

fn default_batch_max_export_batch_size() -> u32 {
    512
}

fn default_batch_max_queue_size() -> u32 {
    2048
}

fn default_batch_max_export_timeout() -> Duration {
    Duration::from_secs(5)
}

fn default_batch_scheduled_delay() -> Duration {
    Duration::from_secs(5)
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct TracingPropagationConfig {
    #[serde(default = "default_propagation_trace_context")]
    pub trace_context: bool,
    #[serde(default = "default_propagation_baggage")]
    pub baggage: bool,
}

impl Default for TracingPropagationConfig {
    fn default() -> Self {
        Self {
            trace_context: default_propagation_trace_context(),
            baggage: default_propagation_baggage(),
        }
    }
}

fn default_propagation_trace_context() -> bool {
    false
}
fn default_propagation_baggage() -> bool {
    false
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub enum OtlpProtocol {
    #[serde(rename = "grpc")]
    Grpc,
    #[serde(rename = "http")]
    Http,
}
