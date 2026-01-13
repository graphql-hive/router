use once_cell::sync::Lazy;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    primitives::value_or_expression::ValueOrExpression,
    telemetry::tracing::{BatchProcessorConfig, OtlpGrpcConfig, OtlpHttpConfig, OtlpProtocol},
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
            batch_processor: BatchProcessorConfig::default(),
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
