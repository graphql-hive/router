use hive_router_config::supergraph::SupergraphSource;

use crate::supergraph::{
    base::{LoadSupergraphError, SupergraphLoader},
    file::SupergraphFileLoader,
    hive::SupergraphHiveConsoleLoader,
};
use tracing::debug;

pub mod base;
pub mod file;
pub mod hive;

pub async fn resolve_from_config(
    config: &SupergraphSource,
) -> Result<Box<dyn SupergraphLoader + Send + Sync>, LoadSupergraphError> {
    debug!("Resolving supergraph from config: {:?}", config);

    match config {
        SupergraphSource::File { path } => Ok(SupergraphFileLoader::new(path).await?),
        SupergraphSource::HiveConsole { endpoint, key } => {
            Ok(SupergraphHiveConsoleLoader::new(endpoint, key).await?)
        }
    }
}
