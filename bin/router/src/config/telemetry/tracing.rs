use std::fs;
use std::{path::PathBuf, time::Duration};
use tonic::transport::{Certificate, ClientTlsConfig, Identity};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::config::primitives::value_or_expression::ValueOrExpression;

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
#[derive(Default)]
#[non_exhaustive]
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
        let has_enabled_exporter = self.exporters.iter().any(TracingExporterConfig::is_enabled);
        let has_enabled_datadog = self.exporters.iter().any(
            |exporter| matches!(exporter, TracingExporterConfig::Datadog(config) if config.enabled),
        );

        // datadog still records zero-sampled spans so its all-request
        // statistics stay complete
        has_enabled_exporter && (self.collect.sampling > 0.0 || has_enabled_datadog)
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, Default)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct TracingInstrumentationConfig {
    #[serde(default)]
    pub spans: TracingSpansInstrumentationConfig,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
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
#[non_exhaustive]
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
#[non_exhaustive]
pub struct TracingCollectConfig {
    #[serde(default = "default_max_events_per_span")]
    pub max_events_per_span: u32,
    #[serde(default = "default_max_attributes_per_span")]
    pub max_attributes_per_span: u32,
    #[serde(default = "default_max_attributes_per_event")]
    pub max_attributes_per_event: u32,
    #[serde(default = "default_max_attributes_per_link")]
    pub max_attributes_per_link: u32,
    /// Fraction of traces to retain, from `0.0` to `1.0`.
    ///
    /// This can also be set with `TELEMETRY_TRACING_SAMPLING_RATE`.
    ///
    /// When an enabled Datadog exporter is present, the resolved Router value
    /// becomes Datadog's provider-wide catch-all sample rate and overrides
    /// `DD_TRACE_SAMPLE_RATE`. More specific `DD_TRACE_SAMPLING_RULES`, remote
    /// configuration, and the `DD_TRACE_RATE_LIMIT` retained-trace ceiling still
    /// apply. The ceiling defaults to 100 retained traces per second when
    /// explicit sampling is active, so high traffic can retain a lower percentage
    /// than this value.
    ///
    /// Datadog keeps instrumentation active at `0.0` so it can
    /// compute all-request counts, error rates, and latency statistics without
    /// retaining detailed traces.
    #[serde(default = "default_sampling")]
    pub sampling: f64,
    /// Makes the generic OpenTelemetry sampler respect its parent span's decision.
    ///
    /// This does not configure Datadog. Datadog uses its own native parent-aware
    /// sampler whenever an enabled Datadog exporter owns the shared provider.
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
#[non_exhaustive]
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
#[non_exhaustive]
pub struct OtlpHttpConfig {
    #[serde(default)]
    pub headers: std::collections::HashMap<String, ValueOrExpression<String>>,
}

#[derive(Debug, Default, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct OtlpGrpcConfig {
    #[serde(default)]
    pub metadata: std::collections::HashMap<String, ValueOrExpression<String>>,
    #[serde(default)]
    pub tls: OtlpGrpcTlsConfig,
}

#[derive(Debug, Default, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
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
#[non_exhaustive]
pub enum TracingExporterConfig {
    #[serde(rename = "otlp")]
    Otlp(Box<TracingOtlpConfig>),
    #[serde(rename = "stdout")]
    Stdout(Box<StdoutExporterConfig>),
    /// Exports traces through Datadog's native Rust tracer and a Datadog Agent.
    ///
    /// Enabling this exporter makes Datadog own sampling for the single shared
    /// tracer provider. OTLP, stdout, and Hive exporters can coexist on that
    /// provider, but only one Datadog exporter can be enabled because the native
    /// provider installs one Datadog processor with one Agent configuration.
    #[serde(rename = "datadog")]
    Datadog(Box<DatadogExporterConfig>),
}

/// Configures native Datadog tracing through a Datadog Agent.
///
/// This is different from sending generic OTLP data to Datadog. Native tracing
/// records all requests for Datadog APM request, error, and latency statistics
/// and applies Datadog sampling, rules, rate limiting, remote configuration, and
/// service identity conventions inside the Router process.
///
/// Router resource attributes are passed to Datadog, which applies its native
/// precedence rules for equivalent `DD_SERVICE`, `DD_ENV`, and `DD_VERSION`
/// values.
///
/// Enabling this exporter does not change trace propagation. The Router keeps
/// the propagators configured under `telemetry.tracing.propagation`, including
/// W3C Trace Context. Datadog supports W3C `traceparent` and `tracestate` headers,
/// so Datadog-specific headers are not required when connected services also
/// support W3C propagation.
#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct DatadogExporterConfig {
    /// Enables this Datadog exporter.
    ///
    /// Default: `true`.
    ///
    /// Set this to `false` to disable only Datadog export. Do not use
    /// `DD_TRACE_ENABLED=false` for that purpose: Datadog owns sampling on the
    /// shared provider, so that environment variable disables tracing for every
    /// OTLP, stdout, and Hive processor attached to it.
    #[serde(default = "default_datadog_config_enabled")]
    pub enabled: bool,
    /// Datadog Agent trace endpoint.
    ///
    /// This can be a static value or an expression. It normally uses port
    /// `8126`, not the OTLP gRPC port `4317`. A configured value overrides
    /// `DD_TRACE_AGENT_URL`, `DD_AGENT_HOST`, and `DD_TRACE_AGENT_PORT`.
    ///
    /// When omitted, Datadog resolves the Agent from those native environment
    /// variables and its defaults. The Router validates configured URLs during
    /// startup but does not probe Agent reachability, so an Agent outage does not
    /// prevent the Router from starting.
    #[serde(default)]
    pub endpoint: Option<ValueOrExpression<String>>,
    /// Records and exports the full GraphQL document to Datadog.
    ///
    /// Default: `false`.
    ///
    /// GraphQL documents can contain sensitive literals, so enable this only
    /// after reviewing the data exposure.
    ///
    /// Suppression happens at the shared instrumentation boundary because
    /// Datadog's native processor cannot be wrapped. Consequently, when Datadog
    /// is enabled, the default suppression also removes `graphql.document` from
    /// coexisting OTLP, stdout, and Hive exporters.
    ///
    /// Setting this to `true` restores recording for the shared provider, although
    /// each exporter can still apply its own redaction.
    #[serde(default)]
    pub include_graphql_document: bool,
}

fn default_datadog_config_enabled() -> bool {
    true
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
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
            TracingExporterConfig::Datadog(datadog_config) => datadog_config.enabled,
        }
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
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
#[non_exhaustive]
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
#[non_exhaustive]
pub enum OtlpProtocol {
    #[serde(rename = "grpc")]
    Grpc,
    #[serde(rename = "http")]
    Http,
}
