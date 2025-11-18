use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::primitives::{file_path::FilePath, retry_policy::RetryPolicyConfig};

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields, tag = "source")]
pub enum SupergraphSource {
    /// Loads a supergraph from the filesystem.
    /// The path can be either absolute or relative to the router's working directory.
    #[serde(rename = "file")]
    File {
        /// The path to the supergraph file.
        ///
        /// Can also be set using the `SUPERGRAPH_FILE_PATH` environment variable.
        path: FilePath,
        /// Optional interval at which the file should be polled for changes.
        /// If not provided, the file will only be loaded once when the router starts.
        #[serde(
            default = "default_file_poll_interval",
            deserialize_with = "humantime_serde::deserialize",
            serialize_with = "humantime_serde::serialize"
        )]
        poll_interval: Option<Duration>,
    },
    /// Loads a supergraph from Hive Console CDN.
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
        /// Interval at which the Hive Console should be polled for changes.
        ///
        /// Can also be set using the `HIVE_CDN_POLL_INTERVAL` environment variable.
        #[serde(
            default = "default_hive_poll_interval",
            deserialize_with = "humantime_serde::deserialize",
            serialize_with = "humantime_serde::serialize"
        )]
        poll_interval: Duration,
        /// Request timeout for the Hive Console CDN requests.
        #[serde(
            default = "default_hive_request_timeout",
            deserialize_with = "humantime_serde::deserialize",
            serialize_with = "humantime_serde::serialize"
        )]
        request_timeout: Duration,
        /// Connect timeout for the Hive Console CDN requests.
        #[serde(
            default = "default_hive_connect_timeout",
            deserialize_with = "humantime_serde::deserialize",
            serialize_with = "humantime_serde::serialize"
        )]
        connect_timeout: Duration,
        /// Whether to accept invalid TLS certificates when connecting to the Hive Console CDN.
        #[serde(default = "default_accept_invalid_certs")]
        accept_invalid_certs: bool,
        /// Interval at which the Hive Console should be polled for changes.
        ///
        /// By default, an exponential backoff retry policy is used, with 10 attempts.
        #[serde(default = "default_hive_retry_policy")]
        retry_policy: RetryPolicyConfig,
    },
}

fn default_accept_invalid_certs() -> bool {
    false
}

fn default_hive_request_timeout() -> Duration {
    Duration::from_secs(60)
}

fn default_hive_connect_timeout() -> Duration {
    Duration::from_secs(10)
}

fn default_hive_retry_policy() -> RetryPolicyConfig {
    RetryPolicyConfig { max_retries: 10 }
}

impl SupergraphSource {
    pub fn source_name(&self) -> &str {
        match self {
            SupergraphSource::File { .. } => "file",
            SupergraphSource::HiveConsole { .. } => "hive",
        }
    }
}

fn default_hive_poll_interval() -> Duration {
    Duration::from_secs(10)
}

fn default_file_poll_interval() -> Option<Duration> {
    None
}

impl Default for SupergraphSource {
    fn default() -> Self {
        SupergraphSource::File {
            path: FilePath::new_from_relative("supergraph.graphql")
                .expect("failed to resolve local path for supergraph file source"),
            poll_interval: default_file_poll_interval(),
        }
    }
}
