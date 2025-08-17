use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct HttpExecutorConfig {
    #[serde(default = "default_max_connections_per_host")]
    pub max_connections_per_host: usize,
    #[serde(default = "default_max_idle_conns")]
    pub max_idle_conns: usize,

    #[serde(default = "default_pool_idle_timeout_seconds")]
    pub pool_idle_timeout_seconds: u64,

    #[serde(default = "default_dedupe_enabled")]
    pub dedupe_enabled: bool,
    #[serde(default = "default_dedupe_fingerprint_headers")]
    pub dedupe_fingerprint_headers: Vec<String>,
}

fn default_max_connections_per_host() -> usize {
    100
}

fn default_max_idle_conns() -> usize {
    1024
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

impl Default for HttpExecutorConfig {
    fn default() -> Self {
        Self {
            max_connections_per_host: default_max_connections_per_host(),
            max_idle_conns: default_max_idle_conns(),
            pool_idle_timeout_seconds: default_pool_idle_timeout_seconds(),
            dedupe_enabled: default_dedupe_enabled(),
            dedupe_fingerprint_headers: default_dedupe_fingerprint_headers(),
        }
    }
}
