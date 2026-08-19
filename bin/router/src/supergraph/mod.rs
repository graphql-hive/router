use std::sync::Arc;

use crate::config::supergraph::SupergraphSource;
use crate::telemetry::logging::targets;

use crate::{
    storage::{utils::resolve_value_or_expression, StorageManager},
    supergraph::{
        apollo_graphos::{ApolloGraphOSSupergraphError, SupergraphApolloGraphOSLoader},
        base::{LoadSupergraphError, SupergraphLoader},
        file::{FileSupergraphError, SupergraphFileLoader},
        hive::{HiveConsoleSupergraphError, SupergraphHiveConsoleLoader},
        storage::{StorageSupergraphError, SupergraphStorageLoader},
    },
};
use tracing::debug;

pub mod apollo_graphos;
pub mod base;
pub mod file;
pub mod hive;
pub mod storage;

pub fn resolve_from_config(
    config: &SupergraphSource,
    storage_manager: Arc<StorageManager>,
) -> Result<Box<dyn SupergraphLoader + Send + Sync>, LoadSupergraphError> {
    debug!(
      target: targets::SUPERGRAPH,
      source = config.source_name(),
      "Creating supergraph loader",
    );

    match config {
        SupergraphSource::File {
            path,
            poll_interval,
        } => {
            let path = path
                .as_ref()
                .ok_or(FileSupergraphError::MissingSupergraphFilePath)?;
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
                .ok_or(HiveConsoleSupergraphError::MissingHiveCDNEndpoint)?;
            let key = key
                .as_ref()
                .ok_or(HiveConsoleSupergraphError::MissingHiveCDNKey)?;

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
        SupergraphSource::ApolloGraphOS {
            graph_ref,
            key,
            endpoint,
            timeout,
            accept_invalid_certs,
        } => {
            let graph_ref = graph_ref
                .as_ref()
                .ok_or(ApolloGraphOSSupergraphError::MissingApolloGraphRef)?;
            let key = key
                .as_ref()
                .ok_or(ApolloGraphOSSupergraphError::MissingApolloKey)?;

            Ok(SupergraphApolloGraphOSLoader::try_new(
                endpoint.clone().into(),
                graph_ref,
                key,
                *timeout,
                *accept_invalid_certs,
            )?)
        }
        SupergraphSource::Storage {
            storage_id,
            location,
            poll_interval,
        } => match storage_manager.get_storage_runtime(storage_id) {
            None => Err(StorageSupergraphError::StorageIdNotFound(storage_id.to_string()).into()),
            Some(runtime) => {
                let location = resolve_value_or_expression(location, "supergraph.storage.key")
                    .map_err(StorageSupergraphError::from)?;

                Ok(SupergraphStorageLoader::try_new(
                    runtime.clone(),
                    location,
                    *poll_interval,
                )?)
            }
        },
        // there is no loader for a source that's entirely plugin-provided, this should never
        // be called in when the `supergraph.source = plugin`
        SupergraphSource::Plugin => Err(LoadSupergraphError::NoLoaderForPluginSource),
    }
}
