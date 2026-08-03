use async_trait::async_trait;

use crate::supergraph::{
    apollo_graphos::ApolloGraphOSSupergraphError, file::FileSupergraphError,
    hive::HiveConsoleSupergraphError, storage::StorageSupergraphError,
};

#[derive(Debug, thiserror::Error)]
pub enum LoadSupergraphError {
    #[error(transparent)]
    File(#[from] FileSupergraphError),
    #[error(transparent)]
    HiveConsole(#[from] HiveConsoleSupergraphError),
    #[error(transparent)]
    ApolloGraphOS(#[from] ApolloGraphOSSupergraphError),
    #[error(transparent)]
    Storage(#[from] StorageSupergraphError),
    #[error("'supergraph.source: plugin' has no loader - a plugin must select a supergraph for every request")]
    NoLoaderForPluginSource,
}

#[derive(Debug)]
pub enum ReloadSupergraphResult {
    Unchanged,
    Changed { new_sdl: String },
}

#[async_trait]
pub trait SupergraphLoader {
    async fn load(&self) -> Result<ReloadSupergraphResult, LoadSupergraphError>;
    fn reload_interval(&self) -> Option<std::time::Duration> {
        None
    }
}
