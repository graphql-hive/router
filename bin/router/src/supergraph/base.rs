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
