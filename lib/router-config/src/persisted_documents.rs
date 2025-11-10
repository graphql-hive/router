use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::primitives::file_path::FilePath;

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct PersistedDocumentsConfig {
    #[serde(default = "default_enabled")]
    /// Whether persisted operations are enabled.
    enabled: bool,
    
    /// Whether to allow arbitrary operations that are not persisted.
    #[serde(default = "default_allow_arbitrary_operations")]
    pub allow_arbitrary_operations: bool,

    /// The source of persisted documents.
    #[serde(default = "default_source")]
    pub source: PersistedDocumentsSource,

    /// The specification to extract persisted operations.
    #[serde(default = "default_spec")]
    pub spec: PersistedDocumentsSpec,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub enum PersistedDocumentsSource {
    #[serde(rename = "file")]
    File {
        /// The path to the file containing persisted operations.
        path: FilePath,
    },
    #[serde(rename = "hive")]
    HiveConsole {
        /// The CDN endpoint from Hive Console target.
        ///
        /// Can also be set using the `HIVE_CDN_ENDPOINT` environment variable.
        endpoint: String,
        /// The CDN Access Token with from the Hive Console target.
        ///
        /// Can also be set using the `HIVE_CDN_KEY` environment variable.
        key: String,
        /// Request timeout for the Hive Console CDN requests.
        #[serde(
            default = "default_hive_request_timeout",
            deserialize_with = "humantime_serde::deserialize",
            serialize_with = "humantime_serde::serialize"
        )]
        request_timeout: Duration,
        /// Connection timeout for the Hive Console CDN requests.
        #[serde(
            default = "default_hive_connect_timeout",
            deserialize_with = "humantime_serde::deserialize",
            serialize_with = "humantime_serde::serialize"
        )]
        connect_timeout: Duration,
        /// Interval at which the Hive Console should be retried upon failure.
        ///
        /// By default, an exponential backoff retry policy is used, with 3 attempts.
        #[serde(default = "default_hive_retry_count")]
        retry_count: u32,
        /// Accept invalid SSL certificates
        /// default: false
        #[serde(default = "default_accept_invalid_certs")]
        accept_invalid_certs: bool,

        /// Configuration for the size of the in-memory caching of persisted documents.
        #[serde(default = "default_cache_size")]
        cache_size: u64,
    },
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(rename_all = "lowercase")]
#[serde(deny_unknown_fields)]
pub enum PersistedDocumentsSpec {
    Hive,
    Apollo,
    Relay,
    Expression(String),
}

fn default_enabled() -> bool {
    false
}

fn default_allow_arbitrary_operations() -> bool {
    false
}

fn default_source() -> PersistedDocumentsSource {
    PersistedDocumentsSource::HiveConsole {
        endpoint: "".into(),
        key: "".into(),
        request_timeout: default_hive_request_timeout(),
        connect_timeout: default_hive_connect_timeout(),
        retry_count: default_hive_retry_count(),
        accept_invalid_certs: default_accept_invalid_certs(),
        cache_size: default_cache_size(),
    }
}

fn default_spec() -> PersistedDocumentsSpec {
    PersistedDocumentsSpec::Hive
}

fn default_hive_request_timeout() -> Duration {
    Duration::from_secs(15)
}

fn default_hive_connect_timeout() -> Duration {
    Duration::from_secs(5)
}

fn default_hive_retry_count() -> u32 {
    3
}

fn default_accept_invalid_certs() -> bool {
    false
}

fn default_cache_size() -> u64 {
    1000
}