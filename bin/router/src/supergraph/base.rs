use async_trait::async_trait;

#[derive(Debug, thiserror::Error)]
pub enum LoadSupergraphError {
    #[error("Failed to read supergraph file: {0}")]
    ReadFileError(#[from] std::io::Error),
    #[error("Failed to read supergraph from network: {0}")]
    NetworkError(#[from] reqwest::Error),
}

#[async_trait]
pub trait SupergraphLoader {
    async fn reload(&mut self) -> Result<(), LoadSupergraphError>;
    fn init_watcher(&self) -> bool {
        false
    }
    fn current(&self) -> Option<&str>;
}
