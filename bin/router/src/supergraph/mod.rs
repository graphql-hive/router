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

pub fn resolve_from_config(
    config: &SupergraphSource,
) -> Result<Box<dyn SupergraphLoader + Send + Sync>, LoadSupergraphError> {
    debug!(
        "Creating supergraph loader from source {}",
        config.source_name()
    );

    match config {
        SupergraphSource::File {
            path,
            poll_interval,
        } => {
            let path = path
                .as_ref()
                .ok_or(LoadSupergraphError::MissingSupergraphFilePath)?;
            Ok(SupergraphFileLoader::new(path, *poll_interval)?)
        }
        SupergraphSource::HiveConsole {
            endpoint,
            key,
            connect_timeout,
            request_timeout,
            accept_invalid_certs,
            retry_policy,
            poll_interval,
        } => {
            let endpoint = endpoint
                .as_ref()
                .ok_or(LoadSupergraphError::MissingHiveCDNEndpoint)?;
            let key = key.as_ref().ok_or(LoadSupergraphError::MissingHiveCDNKey)?;

            Ok(SupergraphHiveConsoleLoader::try_new(
                endpoint.clone(),
                key,
                *poll_interval,
                *connect_timeout,
                *request_timeout,
                *accept_invalid_certs,
                retry_policy.max_retries,
            )?)
        }
    }
}
