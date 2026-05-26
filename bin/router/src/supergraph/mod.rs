use std::sync::Arc;

use hive_router_config::supergraph::SupergraphSource;

use crate::{
    storage::{utils::resolve_value_or_expression, StorageManager},
    supergraph::{
        base::{LoadSupergraphError, SupergraphLoader},
        file::SupergraphFileLoader,
        hive::SupergraphHiveConsoleLoader,
        storage::SupergraphStorageLoader,
    },
};
use tracing::debug;

pub mod base;
pub mod file;
pub mod hive;
pub mod storage;

pub fn resolve_from_config(
    config: &SupergraphSource,
    storage_manager: Arc<StorageManager>,
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
                endpoint.clone().into(),
                key,
                *poll_interval,
                *connect_timeout,
                *request_timeout,
                *accept_invalid_certs,
                retry_policy.max_retries,
            )?)
        }
        SupergraphSource::Storage {
            storage_id,
            location,
            poll_interval,
        } => {
            match storage_manager.get_storage_runtime(storage_id) {
                None => Err(LoadSupergraphError::StorageIdNotFound(
                    storage_id.to_string(),
                )),
                Some(runtime) => {
                    let location = resolve_value_or_expression(location, "supergraph.storage.key")?;

                    Ok(SupergraphStorageLoader::try_new(
                        runtime.clone(),
                        location,
                        *poll_interval,
                    )?)
                }
            }

            // Ok(SupergraphHiveConsoleLoader::try_new(
            //     endpoint.clone().into(),
            //     key,
            //     *poll_interval,
            //     *connect_timeout,
            //     *request_timeout,
            //     *accept_invalid_certs,
            //     retry_policy.max_retries,
            // )?)
        }
    }
}
