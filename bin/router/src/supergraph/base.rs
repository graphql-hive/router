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
