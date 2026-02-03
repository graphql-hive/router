use std::time::Duration;

use once_cell::sync::Lazy;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    primitives::value_or_expression::ValueOrExpression,
    telemetry::tracing::{OtlpGrpcConfig, OtlpHttpConfig, OtlpProtocol},
    usage_reporting::UsageReportingConfig,
};

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
#[derive(Default)]
pub struct HiveTelemetryConfig {
    #[serde(default = "default_hive_tracing_endpoint")]
    pub endpoint: ValueOrExpression<String>,
    /// Your [Registry Access Token](https://the-guild.dev/graphql/hive/docs/management/targets#registry-access-tokens) with write permission.
    #[serde(default)]
    pub token: Option<ValueOrExpression<String>>,
    /// A target ID, this can either be a slug following the format “$organizationSlug/$projectSlug/$targetSlug” (e.g “the-guild/graphql-hive/staging”) or an UUID (e.g. “a0f4c605-6541-4350-8cfe-b31f21a4bf80”). To be used when the token is configured with an organization access token.
    #[serde(default)]
    pub target: Option<ValueOrExpression<String>>,
    #[serde(default)]
    pub tracing: TracingConfig,
    #[serde(default)]
    pub usage_reporting: UsageReportingConfig,
}

// Target ID regexp for validation: slug format
static TARGET_ID_SLUG_REGEX: Lazy<regex_automata::meta::Regex> = Lazy::new(|| {
    regex_automata::meta::Regex::new(r"^[a-zA-Z0-9-_]+\/[a-zA-Z0-9-_]+\/[a-zA-Z0-9-_]+$")
        .expect("Failed to compile slug regex")
});
// Target ID regexp for validation: UUID format
static TARGET_ID_UUID_REGEX: Lazy<regex_automata::meta::Regex> = Lazy::new(|| {
    regex_automata::meta::Regex::new(
        r"^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$",
    )
    .expect("Failed to compile UUID regex")
});

pub fn is_uuid_target_ref(target_id: &str) -> bool {
    TARGET_ID_UUID_REGEX.is_match(target_id.trim())
}

pub fn is_slug_target_ref(target_id: &str) -> bool {
    TARGET_ID_SLUG_REGEX.is_match(target_id.trim())
}

fn default_hive_tracing_endpoint() -> ValueOrExpression<String> {
    ValueOrExpression::Value("https://api.graphql-hive.com/otel/v1/traces".to_string())
}

impl Default for TracingConfig {
    fn default() -> Self {
        Self {
            enabled: default_tracing_enabled(),
            batch_processor: TraceBatchProcessorConfig::default(),
            protocol: OtlpProtocol::Http,
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
    pub batch_processor: TraceBatchProcessorConfig,
    pub protocol: OtlpProtocol,
    #[serde(default)]
    pub http: Option<OtlpHttpConfig>,
    #[serde(default)]
    pub grpc: Option<OtlpGrpcConfig>,
}

fn default_tracing_enabled() -> bool {
    true
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct TraceBatchProcessorConfig {
    /// Maximum number of unique traces to keep in memory simultaneously.
    ///
    /// If this limit is reached, the processor will attempt to flush ready traces.
    /// If no traces are ready, new spans for new traces will be dropped to preserve memory.
    /// Spans for existing traces will still be accepted.
    #[serde(default = "default_max_traces_in_memory")]
    pub max_traces_in_memory: u32,

    /// Maximum number of spans to buffer per single trace.
    ///
    /// If a trace exceeds this limit, subsequent spans for that trace will be dropped.
    #[serde(default = "default_max_spans_per_trace")]
    pub max_spans_per_trace: u32,

    /// Maximum time to wait for the exporter to finish a batch export.
    #[serde(
        default = "default_max_export_timeout",
        deserialize_with = "humantime_serde::deserialize",
        serialize_with = "humantime_serde::serialize"
    )]
    #[schemars(with = "String")]
    pub max_export_timeout: Duration,

    /// Capacity of the input channel (from `on_end` to the worker thread).
    #[serde(default = "default_max_queue_size")]
    pub max_queue_size: u32,

    /// Maximum number of traces (not spans) to include in a single export batch.
    #[serde(default = "default_max_export_batch_size")]
    pub max_export_batch_size: u32,

    /// Maximum time to wait before exporting ready traces if the batch size
    /// hasn't been reached.
    #[serde(
        default = "default_scheduled_delay",
        deserialize_with = "humantime_serde::deserialize",
        serialize_with = "humantime_serde::serialize"
    )]
    #[schemars(with = "String")]
    pub scheduled_delay: Duration,

    /// Maximum number of export tasks that can run concurrently.
    #[serde(default = "default_max_concurrent_exports")]
    pub max_concurrent_exports: u32,
}

impl Default for TraceBatchProcessorConfig {
    fn default() -> Self {
        Self {
            max_concurrent_exports: default_max_concurrent_exports(),
            max_traces_in_memory: default_max_traces_in_memory(),
            max_export_timeout: default_max_export_timeout(),
            max_spans_per_trace: default_max_spans_per_trace(),
            max_queue_size: default_max_queue_size(),
            max_export_batch_size: default_max_export_batch_size(),
            scheduled_delay: default_scheduled_delay(),
        }
    }
}

fn default_max_traces_in_memory() -> u32 {
    30_000
}

fn default_max_spans_per_trace() -> u32 {
    1000
}

fn default_max_export_timeout() -> Duration {
    Duration::from_secs(5)
}

fn default_max_queue_size() -> u32 {
    20_000
}

fn default_max_export_batch_size() -> u32 {
    500
}

fn default_scheduled_delay() -> Duration {
    Duration::from_secs(5)
}

fn default_max_concurrent_exports() -> u32 {
    1
}
