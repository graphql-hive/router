use std::collections::HashMap;
use std::time::Duration;

use human_size::Size;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::primitives::toggle::ToggleWith;
use crate::primitives::value_or_expression::ValueOrExpression;
use crate::telemetry::{tracing::OtlpGrpcConfig, tracing::OtlpHttpConfig, tracing::OtlpProtocol};

/// Configures metrics collection, processing, and export.
#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct MetricsConfig {
    /// List of metrics exporters.
    ///
    /// Metrics are enabled when at least one exporter is configured and enabled.
    #[serde(default)]
    pub exporters: Vec<MetricsExporterConfig>,
    /// Controls metrics instrumentation behavior, such as histogram aggregation.
    #[serde(default)]
    pub instrumentation: MetricsInstrumentationConfig,
}

impl MetricsConfig {
    pub fn is_enabled(&self) -> bool {
        self.exporters.iter().any(|exporter| exporter.is_enabled())
    }
}

fn default_metrics_interval() -> Duration {
    Duration::from_secs(60)
}

fn default_metrics_max_export_timeout() -> Duration {
    Duration::from_secs(5)
}

/// Defines how metric values accumulate across collection cycles.
#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, Copy, Default)]
#[serde(rename_all = "snake_case")]
pub enum MetricsTemporality {
    /// A measurement interval that continues to expand forward in time from a
    /// starting point.
    ///
    /// New measurements are added to all previous measurements since a start time.
    #[default]
    Cumulative,
    /// A measurement interval that resets each cycle.
    ///
    /// Measurements from one cycle are recorded independently, measurements from
    /// other cycles do not affect them.
    Delta,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct MetricsInstrumentationConfig {
    #[serde(default)]
    pub common: MetricsCommonConfig,
    #[serde(default)]
    pub instruments: MetricsInstrumentsConfig,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct MetricsCommonConfig {
    #[serde(default)]
    pub histogram: MetricsHistogramConfig,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields, tag = "aggregation", rename_all = "snake_case")]
pub enum MetricsHistogramConfig {
    Explicit {
        #[serde(default = "default_explicit_histogram_seconds")]
        seconds: MetricsExplicitHistogramUnitConfig,
        #[serde(default = "default_explicit_histogram_bytes")]
        bytes: MetricsExplicitHistogramUnitConfig,
    },
    Exponential {
        max_size: u32,
        max_scale: i8,
        #[serde(default)]
        record_min_max: bool,
    },
}

impl Default for MetricsHistogramConfig {
    fn default() -> Self {
        Self::Explicit {
            seconds: default_explicit_histogram_seconds(),
            bytes: default_explicit_histogram_bytes(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct MetricsExplicitHistogramUnitConfig {
    pub buckets: MetricsHistogramBuckets,
    #[serde(default)]
    pub record_min_max: bool,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(untagged)]
pub enum MetricsHistogramBuckets {
    Numeric(Vec<f64>),
    HumanReadable(Vec<String>),
}

impl MetricsExplicitHistogramUnitConfig {
    pub fn resolve_seconds_buckets(&self) -> Result<Vec<f64>, String> {
        match &self.buckets {
            MetricsHistogramBuckets::Numeric(values) => Ok(values.clone()),
            MetricsHistogramBuckets::HumanReadable(values) => values
                .iter()
                .map(|value| {
                    humantime::parse_duration(value)
                        .map(|duration| duration.as_secs_f64())
                        .map_err(|err| {
                            format!("Invalid duration bucket '{value}' in seconds.buckets: {err}")
                        })
                })
                .collect(),
        }
    }

    pub fn resolve_bytes_buckets(&self) -> Result<Vec<f64>, String> {
        match &self.buckets {
            MetricsHistogramBuckets::Numeric(values) => Ok(values.clone()),
            MetricsHistogramBuckets::HumanReadable(values) => values
                .iter()
                .map(|value| {
                    value
                        .parse::<Size>()
                        .map(|size| size.to_bytes() as f64)
                        .map_err(|err| {
                            format!("Invalid byte bucket '{value}' in bytes.buckets: {err}")
                        })
                })
                .collect(),
        }
    }
}

fn default_explicit_histogram_seconds_buckets() -> Vec<f64> {
    vec![
        0.005, 0.01, 0.025, 0.05, 0.075, 0.1, 0.25, 0.5, 0.75, 1.0, 2.5, 5.0, 7.5, 10.0,
    ]
}

fn default_explicit_histogram_bytes_buckets() -> Vec<f64> {
    vec![
        128.0, 512.0, 1024.0, 2048.0, 4096.0, 8192.0, 16384.0, 32768.0, 65536.0, 131072.0,
        262144.0, 524288.0, 1048576.0, 2097152.0, 3145728.0, 4194304.0, 5242880.0,
    ]
}

fn default_explicit_histogram_seconds() -> MetricsExplicitHistogramUnitConfig {
    MetricsExplicitHistogramUnitConfig {
        buckets: MetricsHistogramBuckets::Numeric(default_explicit_histogram_seconds_buckets()),
        record_min_max: false,
    }
}

fn default_explicit_histogram_bytes() -> MetricsExplicitHistogramUnitConfig {
    MetricsExplicitHistogramUnitConfig {
        buckets: MetricsHistogramBuckets::Numeric(default_explicit_histogram_bytes_buckets()),
        record_min_max: false,
    }
}

pub type MetricsInstrumentsConfig = HashMap<String, ToggleWith<InstrumentConfig>>;

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, Default, PartialEq)]
pub struct InstrumentConfig {
    pub attributes: HashMap<String, bool>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields, tag = "kind")]
pub enum MetricsExporterConfig {
    #[serde(rename = "otlp")]
    Otlp(Box<MetricsOtlpConfig>),
    #[serde(rename = "prometheus")]
    Prometheus(Box<MetricsPrometheusConfig>),
}

impl MetricsExporterConfig {
    fn is_enabled(&self) -> bool {
        match self {
            MetricsExporterConfig::Otlp(config) => config.enabled,
            MetricsExporterConfig::Prometheus(config) => config.enabled,
        }
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct MetricsOtlpConfig {
    /// Enables or disables this OTLP metrics exporter.
    ///
    /// Default: `true`.
    #[serde(default = "default_otlp_config_enabled")]
    pub enabled: bool,
    /// OTLP endpoint URL.
    ///
    /// Can be a static value or an expression.
    #[serde(default)]
    pub endpoint: ValueOrExpression<String>,
    /// Transport protocol used for OTLP metrics export.
    pub protocol: OtlpProtocol,
    /// Interval between periodic metric export attempts.
    ///
    /// Default: `60s`.
    #[serde(
        default = "default_metrics_interval",
        deserialize_with = "humantime_serde::deserialize",
        serialize_with = "humantime_serde::serialize"
    )]
    #[schemars(with = "String")]
    pub interval: Duration,
    /// Aggregation temporality used for this OTLP exporter.
    ///
    /// Default: `cumulative`.
    #[serde(default)]
    pub temporality: MetricsTemporality,
    /// Maximum time allowed for a single metrics export attempt.
    ///
    /// Default: `5s`.
    #[serde(
        default = "default_metrics_max_export_timeout",
        deserialize_with = "humantime_serde::deserialize",
        serialize_with = "humantime_serde::serialize"
    )]
    #[schemars(with = "String")]
    pub max_export_timeout: Duration,
    /// HTTP-specific OTLP settings.
    #[serde(default)]
    pub http: Option<OtlpHttpConfig>,
    /// gRPC-specific OTLP settings.
    #[serde(default)]
    pub grpc: Option<OtlpGrpcConfig>,
}

fn default_otlp_config_enabled() -> bool {
    true
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct MetricsPrometheusConfig {
    #[serde(default = "default_prometheus_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default = "default_prometheus_path")]
    pub path: String,
}

fn default_prometheus_enabled() -> bool {
    true
}

fn default_prometheus_path() -> String {
    "/metrics".to_string()
}
