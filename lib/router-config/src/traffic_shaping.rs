use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct TrafficShapingConfig {
    /// Limits the concurrent amount of requests/connections per host/subgraph.
    #[serde(default = "default_max_connections_per_host")]
    pub max_connections_per_host: usize,

    /// Timeout for idle sockets being kept-alive.
    #[serde(
        default = "default_pool_idle_timeout",
        deserialize_with = "humantime_serde::deserialize",
        serialize_with = "humantime_serde::serialize"
    )]
    #[schemars(with = "String")]
    pub pool_idle_timeout: Duration,

    /// Enables/disables request deduplication to subgraphs.
    ///
    /// When requests exactly matches the hashing mechanism (e.g., subgraph name, URL, headers, query, variables), and are executed at the same time, they will
    /// be deduplicated by sharing the response of other in-flight requests.
    #[serde(default = "default_dedupe_enabled")]
    pub dedupe_enabled: bool,
}

impl Default for TrafficShapingConfig {
    fn default() -> Self {
        Self {
            max_connections_per_host: default_max_connections_per_host(),
            pool_idle_timeout: default_pool_idle_timeout(),
            dedupe_enabled: default_dedupe_enabled(),
        }
    }
}

fn default_max_connections_per_host() -> usize {
    100
}

fn default_pool_idle_timeout() -> Duration {
    Duration::from_secs(50)
}

fn default_dedupe_enabled() -> bool {
    true
}
