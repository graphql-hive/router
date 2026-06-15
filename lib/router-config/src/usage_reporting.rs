use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::headers::OneOrMany;
use crate::primitives::percentage::Percentage;

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(untagged)]
pub enum UsageReportingExclude {
    Expression { expression: String },
    OperationNames(Vec<String>),
}

#[derive(Debug, Default, Deserialize, Serialize, JsonSchema, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UsageReportingSamplingKeyKind {
    #[default]
    OperationName,
    OperationType,
    OperationBody,
}

pub type UsageReportingSamplingKey = OneOrMany<UsageReportingSamplingKeyKind>;

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct AtLeastOnceSamplingConfig {
    /// The key used for at-least-once sampling, to determine unique operations.
    ///
    /// Possible values:
    ///  - `operation_name`: the name of the GraphQL operation
    ///  - `operation_type`: the type
    ///  - `operation_body`: the body
    ///
    ///
    /// You can also provide multiple values. In that case, the router combines them
    /// into one key.
    ///
    /// No default value.
    pub key: UsageReportingSamplingKey,

    #[serde(default = "default_max_distinct_keys")]
    /// Maximum number of unique keys kept in memory for at-least-once sampling.
    /// When the limit is reached, older keys may be removed.
    ///
    /// Every key consumes 16 bytes of memory.
    ///
    /// Defaults to 100k.
    pub max_distinct_keys: u64,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct UsageReportingSamplingConfig {
    #[serde(default = "default_sample_rate")]
    #[schemars(with = "String")]
    pub rate: Percentage,

    #[serde(default)]
    /// At-least-once sampling configuration.
    ///
    /// Used together with `rate`.
    /// The first request for each unique key is always sampled.
    /// Later requests for the same key are sampled using the configured rate.
    ///
    /// The distinct key is built from the `key` field.
    ///
    /// Disabled by default.
    pub at_least_once: Option<AtLeastOnceSamplingConfig>,
}

impl Default for UsageReportingSamplingConfig {
    fn default() -> Self {
        Self {
            rate: default_sample_rate(),
            at_least_once: None,
        }
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct UsageReportingConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,

    /// For self-hosting, you can override `/usage` endpoint (defaults to `https://app.graphql-hive.com/usage`).
    #[serde(default = "default_endpoint")]
    pub endpoint: String,

    /// Sample rate to determine sampling.
    /// 0% = never being sent
    /// 50% = half of the requests being sent
    /// 100% = always being sent
    /// Default: 100%
    #[serde(default)]
    pub sampling: UsageReportingSamplingConfig,

    /// An expression in VRL to exclude certain operations from being sent to Hive Console.
    /// Returning `true` from this expression will exclude the operation, while `false` will include it.
    /// This expression is a VRL expression that has access to the request and operation details;
    ///
    /// ```vrl
    ///  if (.request.operation.name == "ExcludeMe") {
    ///    true
    ///  } else {
    ///    false
    ///  }
    /// ```
    /// Backward compatible with both:
    /// - an expression object: `{ expression: "..." }`
    /// - a list of operation names
    #[serde(default)]
    pub exclude: Option<UsageReportingExclude>,

    /// A maximum number of operations to hold in a buffer before sending to Hive Console
    /// Default: 1000
    #[serde(default = "default_buffer_size")]
    pub buffer_size: usize,

    /// Accepts invalid SSL certificates
    /// Default: false
    #[serde(default = "default_accept_invalid_certs")]
    pub accept_invalid_certs: bool,

    /// A timeout for only the connect phase of a request to Hive Console
    /// Default: 5 seconds
    #[serde(
        default = "default_connect_timeout",
        deserialize_with = "humantime_serde::deserialize",
        serialize_with = "humantime_serde::serialize"
    )]
    #[schemars(with = "String")]
    pub connect_timeout: Duration,

    /// A timeout for the entire request to Hive Console
    /// Default: 15 seconds
    #[serde(
        default = "default_request_timeout",
        deserialize_with = "humantime_serde::deserialize",
        serialize_with = "humantime_serde::serialize"
    )]
    #[schemars(with = "String")]
    pub request_timeout: Duration,

    /// Frequency of flushing the buffer to the server
    /// Default: 5 seconds
    #[serde(
        default = "default_flush_interval",
        deserialize_with = "humantime_serde::deserialize",
        serialize_with = "humantime_serde::serialize"
    )]
    #[schemars(with = "String")]
    pub flush_interval: Duration,
}

#[cfg(test)]
mod tests {
    use super::UsageReportingConfig;
    use crate::usage_reporting::UsageReportingExclude;

    #[test]
    fn exclude_supports_expression_object() {
        let config: UsageReportingConfig = serde_json::from_str(
            r#"{
                "enabled": true,
                "sampling": { "rate": "100%" },
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
                "sampling": { "rate": "100%" },
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
    fn at_least_once_no_default() {
        let config = serde_json::from_str::<UsageReportingConfig>(
            r#"{
                "enabled": true,
                "sampling": {
                    "rate": "10%",
                    "at_least_once": {}
                }
            }"#,
        );

        assert!(
            config.is_err(),
            "config with no key should fail to deserialize"
        );
    }
}

impl Default for UsageReportingConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            endpoint: default_endpoint(),
            sampling: Default::default(),
            exclude: None,
            buffer_size: default_buffer_size(),
            accept_invalid_certs: default_accept_invalid_certs(),
            connect_timeout: default_connect_timeout(),
            request_timeout: default_request_timeout(),
            flush_interval: default_flush_interval(),
        }
    }
}

fn default_enabled() -> bool {
    false
}

fn default_endpoint() -> String {
    "https://app.graphql-hive.com/usage".to_string()
}

fn default_sample_rate() -> Percentage {
    Percentage::from_f64(1.0).unwrap()
}

fn default_max_distinct_keys() -> u64 {
    100_000
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
