use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::primitives::percentage::Percentage;

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
    #[serde(default = "default_sample_rate")]
    #[schemars(with = "String")]
    pub sample_rate: Percentage,

    /// A list of operations (by name) to be ignored by Hive.
    /// Example: ["IntrospectionQuery", "MeQuery"]
    #[serde(default)]
    pub exclude: Vec<String>,

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

impl Default for UsageReportingConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            endpoint: default_endpoint(),
            sample_rate: default_sample_rate(),
            exclude: Vec::new(),
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
