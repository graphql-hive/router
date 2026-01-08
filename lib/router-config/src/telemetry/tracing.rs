use std::fs;
use std::{path::PathBuf, time::Duration};
use tonic::transport::{Certificate, ClientTlsConfig, Identity};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::primitives::value_or_expression::ValueOrExpression;

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
#[derive(Default)]
pub struct TracingConfig {
    #[serde(default)]
    pub collect: TracingCollectConfig,
    #[serde(default)]
    pub exporters: Vec<TracingExporterConfig>,
    #[serde(default)]
    pub propagation: TracingPropagationConfig,
    #[serde(default)]
    pub instrumentation: TracingInstrumentationConfig,
}

impl TracingConfig {
    pub fn is_enabled(&self) -> bool {
        // sampling is set to 0? no nead to enable tracing
        self.collect.sampling > 0.0 &&
        // at least one exporter is enabled
        self.exporters.iter().any(|exporter| exporter.is_enabled())
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct TracingInstrumentationConfig {
    #[serde(default)]
    pub spans: TracingSpansInstrumentationConfig,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct TracingSpansInstrumentationConfig {
    /// Controls which semantic conventions are emitted on spans.
    /// Default: SpecCompliant (only stable attributes).
    #[serde(default = "default_spans_mode")]
    pub mode: SpansSemanticConventionsMode,
}

impl Default for TracingSpansInstrumentationConfig {
    fn default() -> Self {
        Self {
            mode: default_spans_mode(),
        }
    }
}

fn default_spans_mode() -> SpansSemanticConventionsMode {
    SpansSemanticConventionsMode::SpecCompliant
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum SpansSemanticConventionsMode {
    /// Only spec-compliant attributes (http.request.*, http.response.*, url.*, etc).
    SpecCompliant,
    /// Only deprecated attributes (http.*, etc). Mainly for legacy setups.
    Deprecated,
    /// Emit both spec-compliant and deprecated attributes.
    SpecAndDeprecated,
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
    #[serde(default)]
    pub tls: OtlpGrpcTlsConfig,
}

#[derive(Debug, Default, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct OtlpGrpcTlsConfig {
    /// The domain name used to verify the server's TLS certificate.
    pub domain_name: Option<String>,
    ///  The path to the client's private key file.
    pub key: Option<PathBuf>,
    ///  The path to the client's certificate file (PEM format).
    pub cert: Option<PathBuf>,
    ///  The path to the Certificate Authority (CA) certificate file (PEM format) used to verify the server's certificate.
    pub ca: Option<PathBuf>,
}

impl TryFrom<&OtlpGrpcTlsConfig> for tonic::transport::ClientTlsConfig {
    type Error = std::io::Error;

    fn try_from(
        value: &OtlpGrpcTlsConfig,
    ) -> Result<tonic::transport::ClientTlsConfig, Self::Error> {
        let mut tls = ClientTlsConfig::new();

        if let Some(domain) = &value.domain_name {
            tls = tls.domain_name(domain);
        }

        if let Some(ca) = &value.ca {
            let ca_cert = fs::read(ca)?;
            tls = tls.ca_certificate(Certificate::from_pem(ca_cert))
        }

        if let Some(cert) = &value.cert {
            let cert = fs::read(cert)?;
            let key = value
                .key
                .as_ref()
                .map(fs::read)
                .transpose()?
                .unwrap_or_default();
            let identity = Identity::from_pem(cert, key);
            tls = tls.identity(identity);
        }

        Ok(tls)
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields, tag = "kind")]
pub enum TracingExporterConfig {
    #[serde(rename = "otlp")]
    Otlp(Box<TracingOtlpConfig>),
    #[serde(rename = "stdout")]
    Stdout(Box<StdoutExporterConfig>),
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct StdoutExporterConfig {
    #[serde(default = "default_stdout_config_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub batch_processor: BatchProcessorConfig,
}

fn default_stdout_config_enabled() -> bool {
    true
}

impl TracingExporterConfig {
    fn is_enabled(&self) -> bool {
        match self {
            TracingExporterConfig::Otlp(otlp_config) => otlp_config.enabled,
            TracingExporterConfig::Stdout(stdout_config) => stdout_config.enabled,
        }
    }
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
    #[serde(default = "default_propagation_b3")]
    pub b3: bool,
    #[serde(default = "default_propagation_jaeger")]
    pub jaeger: bool,
}

impl Default for TracingPropagationConfig {
    fn default() -> Self {
        Self {
            trace_context: default_propagation_trace_context(),
            baggage: default_propagation_baggage(),
            b3: default_propagation_b3(),
            jaeger: default_propagation_jaeger(),
        }
    }
}

fn default_propagation_trace_context() -> bool {
    true
}
fn default_propagation_baggage() -> bool {
    false
}
fn default_propagation_b3() -> bool {
    false
}
fn default_propagation_jaeger() -> bool {
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
