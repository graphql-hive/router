use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

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
        boundaries: Vec<f64>,
        #[serde(default)]
        record_min_max: bool,
    },
    Exponential {
        #[serde(default = "default_histogram_max_size")]
        max_size: u32,
        #[serde(default = "default_histogram_max_scale")]
        max_scale: i8,
        #[serde(default)]
        record_min_max: bool,
    },
}

impl Default for MetricsHistogramConfig {
    fn default() -> Self {
        Self::Exponential {
            max_size: default_histogram_max_size(),
            max_scale: default_histogram_max_scale(),
            record_min_max: false,
        }
    }
}

fn default_histogram_max_size() -> u32 {
    160
}

fn default_histogram_max_scale() -> i8 {
    20
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
