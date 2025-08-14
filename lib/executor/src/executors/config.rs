use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct HttpExecutorConfig {
    #[serde(default = "default_max_connections_per_host")]
    pub max_connections_per_host: usize,

    #[serde(default = "default_pool_idle_timeout_seconds")]
    pub pool_idle_timeout_seconds: u64,
}

fn default_max_connections_per_host() -> usize {
    100
}

fn default_pool_idle_timeout_seconds() -> u64 {
    50
}

impl Default for HttpExecutorConfig {
    fn default() -> Self {
        Self {
            max_connections_per_host: default_max_connections_per_host(),
            pool_idle_timeout_seconds: default_pool_idle_timeout_seconds(),
        }
    }
}
