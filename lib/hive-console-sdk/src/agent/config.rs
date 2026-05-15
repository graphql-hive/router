//! Configuration types for the Hive Console usage reporting agent.
//!
//! These types are deserialized from YAML/JSON configuration in the
//! `hive-router` and `hive-apollo-router-plugin` crates, and consumed by
//! [`crate::agent::UsageAgent::from_config`] to build a runtime agent.
//!
//! Compilation of VRL expressions (`exclude` and `sampler.key.expression`)
//! happens inside the SDK, so configuration consumers do not need to depend
//! on `vrl` themselves.

use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::primitives::circuit_breaker::CircuitBreakerConfig;
use crate::primitives::percentage::Percentage;
use crate::primitives::retry_policy::RetryPolicyConfig;

pub static DEFAULT_HIVE_USAGE_ENDPOINT: &str = "https://app.graphql-hive.com/usage";

/// Top-level configuration for usage reporting.
#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
pub struct UsageReportingConfig {
    /// Whether usage reporting is enabled.
    #[serde(default = "default_enabled")]
    pub enabled: bool,

    /// For self-hosting, you can override the `/usage` endpoint.
    #[serde(default = "default_endpoint")]
    pub endpoint: String,

    /// Strategy used to decide whether a usage report is sent to Hive Console.
    ///
    /// Two strategies are supported today:
    ///
    /// - `fixed`: samples every operation independently with the configured rate.
    /// - `at_least_once`: guarantees the first occurrence per key is reported,
    ///   then applies `rate` to subsequent occurrences with the same key.
    #[serde(default)]
    pub sampler: SamplerConfig,

    /// How to drop operations before they ever reach the sampler.
    ///
    /// Either a VRL expression that returns a boolean (`true` excludes the
    /// operation, `false` keeps it), or a legacy list of operation names to
    /// exclude. The expression has access to the request and operation
    /// details:
    ///
    /// ```vrl
    /// if (.request.operation.name == "ExcludeMe") {
    ///   true
    /// } else {
    ///   false
    /// }
    /// ```
    #[serde(default)]
    pub exclude: Option<UsageReportingExclude>,

    /// Maximum number of operations to hold in a buffer before flushing to Hive Console.
    #[serde(default = "default_buffer_size")]
    pub buffer_size: usize,

    /// Whether to accept invalid SSL certificates.
    #[serde(default = "default_accept_invalid_certs")]
    pub accept_invalid_certs: bool,

    /// Timeout for only the connect phase of a request to Hive Console.
    #[serde(
        default = "default_connect_timeout",
        deserialize_with = "humantime_serde::deserialize",
        serialize_with = "humantime_serde::serialize"
    )]
    #[schemars(with = "String")]
    pub connect_timeout: Duration,

    /// Timeout for the entire request to Hive Console.
    #[serde(
        default = "default_request_timeout",
        deserialize_with = "humantime_serde::deserialize",
        serialize_with = "humantime_serde::serialize"
    )]
    #[schemars(with = "String")]
    pub request_timeout: Duration,

    /// How often the buffer is flushed to Hive Console.
    #[serde(
        default = "default_flush_interval",
        deserialize_with = "humantime_serde::deserialize",
        serialize_with = "humantime_serde::serialize"
    )]
    #[schemars(with = "String")]
    pub flush_interval: Duration,

    /// Retry policy for sending reports to Hive Console.
    #[serde(default)]
    pub retry_policy: RetryPolicyConfig,

    /// Optional tuning of the circuit breaker applied to outbound calls to Hive Console. When omitted, the SDK uses its default settings (50% error threshold, rolling sample of 5, 30s reset timeout, 10 half-open probes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub circuit_breaker: Option<CircuitBreakerConfig>,
}

impl Default for UsageReportingConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            endpoint: default_endpoint(),
            sampler: SamplerConfig::default(),
            exclude: None,
            buffer_size: default_buffer_size(),
            accept_invalid_certs: default_accept_invalid_certs(),
            connect_timeout: default_connect_timeout(),
            request_timeout: default_request_timeout(),
            flush_interval: default_flush_interval(),
            retry_policy: RetryPolicyConfig::default(),
            circuit_breaker: None,
        }
    }
}

/// How to exclude operations from being reported.
///
/// Either a VRL expression that returns a boolean, or a list of operation
/// names to exclude.
#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(untagged)]
pub enum UsageReportingExclude {
    Expression { expression: String },
    OperationNames(Vec<String>),
}

/// Strategy used to decide whether a usage report is sent to Hive Console.
///
/// - `fixed` samples every operation independently with the configured rate.
/// - `at_least_once` guarantees that the first occurrence per `key` is always
///   reported, and applies `rate` to every subsequent occurrence with the
///   same key.
#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum SamplerConfig {
    /// Probabilistic sampling at a fixed rate.
    ///
    /// 0% = never sampled, 100% = always sampled.
    Fixed {
        #[schemars(with = "String")]
        rate: Percentage,
    },
    /// Always reports the first occurrence per `key`, then applies `rate`
    /// to subsequent occurrences with the same key.
    AtLeastOnce {
        /// How to derive the key that identifies "the same operation".
        /// Defaults to `operation_name`.
        #[serde(default)]
        key: AtLeastOnceKey,
        /// Sample rate applied to every occurrence after the first per `key`.
        /// Defaults to 0%, meaning only the first occurrence per key is sampled.
        #[serde(default = "default_at_least_once_rate")]
        #[schemars(with = "String")]
        rate: Percentage,
        /// Maximum number of distinct keys retained in the in-memory
        /// "seen-once" set used to guarantee the first occurrence per key.
        ///
        /// Once this many distinct keys have been observed, the least
        /// recently used entries are evicted, which means an evicted key
        /// can be treated as a "first occurrence" again. Pick a value that
        /// safely covers the expected cardinality of `key` (operation
        /// names, or whatever the VRL expression resolves to).
        ///
        /// Defaults to `1_000`.
        #[serde(default = "default_at_least_once_max_seen_keys")]
        max_seen_keys: u64,
    },
}

impl Default for SamplerConfig {
    fn default() -> Self {
        Self::Fixed {
            rate: Percentage::from_f64(1.0).expect("100% is a valid percentage"),
        }
    }
}

/// How to compute the key used by `at_least_once` sampling to identify
/// repeated occurrences of the same operation.
///
/// Two shapes are supported in configuration:
///
/// - A constant from [`AtLeastOnceKeyConstant`] written as a bare string,
///   for example `key: operation_name` (the default).
/// - An object `{ expression: "..." }` with a VRL expression that returns
///   the key as a string. The expression has access to the same `request`
///   context used by `usage_reporting.exclude`.
#[derive(Debug, Serialize, Deserialize, JsonSchema, Clone)]
#[serde(untagged)]
pub enum AtLeastOnceKey {
    /// A predefined key derived from the report itself (no VRL needed).
    Constant(AtLeastOnceKeyConstant),
    /// A VRL expression that returns the key as a string.
    Expression { expression: String },
}

impl Default for AtLeastOnceKey {
    fn default() -> Self {
        Self::Constant(AtLeastOnceKeyConstant::default())
    }
}

/// Constant keys that `at_least_once` sampling can use without compiling a
/// VRL expression. Kept as its own enum so unknown values are rejected at
/// deserialization time and the JSON schema enumerates the allowed values.
#[derive(Debug, Default, Serialize, Deserialize, JsonSchema, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum AtLeastOnceKeyConstant {
    /// Use the GraphQL operation name as the key.
    /// Anonymous operations share the empty string as their key.
    #[default]
    OperationName,
}

fn default_enabled() -> bool {
    false
}

fn default_endpoint() -> String {
    DEFAULT_HIVE_USAGE_ENDPOINT.to_string()
}

fn default_at_least_once_rate() -> Percentage {
    Percentage::from_f64(0.0).expect("0% is a valid percentage")
}

fn default_at_least_once_max_seen_keys() -> u64 {
    1_000
}

fn default_buffer_size() -> usize {
    1000
}

fn default_accept_invalid_certs() -> bool {
    false
}

fn default_request_timeout() -> Duration {
    Duration::from_secs(15)
}

fn default_connect_timeout() -> Duration {
    Duration::from_secs(5)
}

fn default_flush_interval() -> Duration {
    Duration::from_secs(5)
}

#[cfg(test)]
mod tests {
    use super::{
        AtLeastOnceKey, AtLeastOnceKeyConstant, SamplerConfig, UsageReportingConfig,
        UsageReportingExclude,
    };

    #[test]
    fn exclude_supports_expression_object() {
        let config: UsageReportingConfig = serde_json::from_str(
            r#"{
                "enabled": true,
                "exclude": { "expression": ".request.operation.name == \"Health\"" }
            }"#,
        )
        .expect("config with expression object should deserialize");

        let exclude = config.exclude.expect("exclude should be present");

        assert!(matches!(exclude, UsageReportingExclude::Expression { .. }));
        if let UsageReportingExclude::Expression { expression } = exclude {
            assert_eq!(
                expression, ".request.operation.name == \"Health\"",
                "expression should match the input"
            );
        }
    }

    #[test]
    fn exclude_supports_legacy_operation_list() {
        let config: UsageReportingConfig = serde_json::from_str(
            r#"{
                "enabled": true,
                "exclude": ["IntrospectionQuery", "HealthCheck"]
            }"#,
        )
        .expect("config with legacy operation list should deserialize");

        let exclude = config.exclude.expect("exclude should be present");
        assert!(matches!(exclude, UsageReportingExclude::OperationNames(_)));
        if let UsageReportingExclude::OperationNames(names) = exclude {
            assert_eq!(
                names,
                vec!["IntrospectionQuery".to_string(), "HealthCheck".to_string()],
                "operation names should match the input"
            );
        }
    }

    #[test]
    fn sampler_defaults_to_fixed_100_percent() {
        let config: UsageReportingConfig = serde_json::from_str(r#"{"enabled": true}"#)
            .expect("config without sampler should deserialize");
        match config.sampler {
            SamplerConfig::Fixed { rate } => assert_eq!(rate.as_f64(), 1.0),
            other => panic!("expected default Fixed sampler, got: {:?}", other),
        }
    }

    #[test]
    fn sampler_fixed_with_explicit_rate() {
        let config: UsageReportingConfig = serde_json::from_str(
            r#"{
                "enabled": true,
                "sampler": { "type": "fixed", "rate": "25%" }
            }"#,
        )
        .expect("fixed sampler should deserialize");
        match config.sampler {
            SamplerConfig::Fixed { rate } => assert_eq!(rate.as_f64(), 0.25),
            other => panic!("expected Fixed, got: {:?}", other),
        }
    }

    #[test]
    fn sampler_at_least_once_with_defaults() {
        let config: UsageReportingConfig = serde_json::from_str(
            r#"{
                "enabled": true,
                "sampler": { "type": "at_least_once" }
            }"#,
        )
        .expect("at_least_once with defaults should deserialize");
        match config.sampler {
            SamplerConfig::AtLeastOnce {
                key,
                rate,
                max_seen_keys,
            } => {
                assert!(matches!(
                    key,
                    AtLeastOnceKey::Constant(AtLeastOnceKeyConstant::OperationName)
                ));
                assert_eq!(rate.as_f64(), 0.0);
                assert_eq!(max_seen_keys, 1_000, "default max_seen_keys must be 1_000",);
            }
            other => panic!("expected AtLeastOnce, got: {:?}", other),
        }
    }

    #[test]
    fn sampler_at_least_once_with_constant_key_and_rate() {
        let config: UsageReportingConfig = serde_json::from_str(
            r#"{
                "enabled": true,
                "sampler": {
                    "type": "at_least_once",
                    "key": "operation_name",
                    "rate": "10%"
                }
            }"#,
        )
        .expect("at_least_once with explicit values should deserialize");
        match config.sampler {
            SamplerConfig::AtLeastOnce { key, rate, .. } => {
                assert!(matches!(
                    key,
                    AtLeastOnceKey::Constant(AtLeastOnceKeyConstant::OperationName)
                ));
                assert_eq!(rate.as_f64(), 0.1);
            }
            other => panic!("expected AtLeastOnce, got: {:?}", other),
        }
    }

    #[test]
    fn sampler_at_least_once_rejects_unknown_constant_key() {
        let result: Result<UsageReportingConfig, _> = serde_json::from_str(
            r#"{
                "enabled": true,
                "sampler": { "type": "at_least_once", "key": "unknown_constant" }
            }"#,
        );
        assert!(
            result.is_err(),
            "unknown sampler.key constants must be rejected"
        );
    }

    #[test]
    fn sampler_at_least_once_with_key_expression() {
        let config: UsageReportingConfig = serde_json::from_str(
            r#"{
                "enabled": true,
                "sampler": {
                    "type": "at_least_once",
                    "key": { "expression": ".request.headers.\"x-tenant\"" },
                    "rate": "50%"
                }
            }"#,
        )
        .expect("at_least_once with key expression should deserialize");
        match config.sampler {
            SamplerConfig::AtLeastOnce { key, rate, .. } => {
                match key {
                    AtLeastOnceKey::Expression { expression } => assert_eq!(
                        expression, ".request.headers.\"x-tenant\"",
                        "expression should round-trip"
                    ),
                    other => panic!("expected Expression key, got: {:?}", other),
                }
                assert_eq!(rate.as_f64(), 0.5);
            }
            other => panic!("expected AtLeastOnce, got: {:?}", other),
        }
    }

    #[test]
    fn sampler_unknown_type_is_rejected() {
        let result: Result<UsageReportingConfig, _> = serde_json::from_str(
            r#"{
                "enabled": true,
                "sampler": { "type": "dynamic", "expression": "true" }
            }"#,
        );
        assert!(result.is_err(), "unknown sampler.type must be rejected");
    }
}
