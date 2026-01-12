use async_trait::async_trait;

#[derive(Debug, thiserror::Error)]
#[allow(clippy::enum_variant_names)]
pub enum LoadSupergraphError {
    #[error("Failed to read supergraph file: {0}")]
    ReadFileError(#[from] std::io::Error),
    #[error("Failed to read supergraph from network: {0}")]
    NetworkError(#[from] reqwest_middleware::Error),
    #[error("Failed to read supergraph from network: {0}")]
    NetworkResponseError(#[from] reqwest::Error),
    #[error("Failed to lock supergraph: {0}")]
    LockError(String),
    #[error("Failed to initialize the loader: {0}")]
    InitializationError(String),
    #[error("Invalid configuration: {0}")]
    InvalidConfiguration(String),
    #[error("Supergraph file path is missing. Please provide it via 'SUPERGRAPH_FILE_PATH' environment variable or under 'supergraph.path' in the configuration.")]
    MissingSupergraphFilePath,
    #[error("Hive CDN endpoint is missing. Please provide it via 'HIVE_CDN_ENDPOINT' environment variable or under 'supergraph.endpoint' in the configuration.")]
    MissingHiveCDNEndpoint,
    #[error("Hive CDN key is missing. Please provide it via 'HIVE_CDN_KEY' environment variable or under 'supergraph.key' in the configuration.")]
    MissingHiveCDNKey,
    #[error("Request rejected by circuit breaker")]
    RejectedByCircuitBreaker,
}

#[derive(Debug)]
pub enum ReloadSupergraphResult {
    Unchanged,
    Changed { new_sdl: String },
}

#[async_trait]
pub trait SupergraphLoader {
    async fn load(&self) -> Result<ReloadSupergraphResult, LoadSupergraphError>;
    fn reload_interval(&self) -> Option<&std::time::Duration> {
        None
    }
}
