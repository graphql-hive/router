use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
pub struct TrafficShapingExecutorConfig {
    /// Limits the concurrent amount of requests/connections per host/subgraph.
    #[serde(default = "default_max_connections_per_host")]
    pub max_connections_per_host: usize,

    /// Timeout for idle sockets being kept-alive.
    #[serde(default = "default_pool_idle_timeout_seconds")]
    pub pool_idle_timeout_seconds: u64,

    /// Enables/disables request deduplication to subgraphs.
    ///
    /// When requests exactly matches the hashing mechanism (e.g., subgraph name, URL, headers, query, variables), and are executed at the same time, they will
    /// be deduplicated by sharing the response of other in-flight requests.
    #[serde(default = "default_dedupe_enabled")]
    pub dedupe_enabled: bool,

    /// A list of headers that should be used to fingerprint requests for deduplication.
    ///
    /// If not provided, the default is to use the "authorization" header only.
    #[serde(default = "default_dedupe_fingerprint_headers")]
    pub dedupe_fingerprint_headers: Vec<String>,
}

impl Default for TrafficShapingExecutorConfig {
    fn default() -> Self {
        Self {
            max_connections_per_host: default_max_connections_per_host(),
            pool_idle_timeout_seconds: default_pool_idle_timeout_seconds(),
            dedupe_enabled: default_dedupe_enabled(),
            dedupe_fingerprint_headers: default_dedupe_fingerprint_headers(),
        }
    }
}

fn default_max_connections_per_host() -> usize {
    100
}

fn default_pool_idle_timeout_seconds() -> u64 {
    50
}

fn default_dedupe_enabled() -> bool {
    true
}

fn default_dedupe_fingerprint_headers() -> Vec<String> {
    vec!["authorization".to_string()]
}
