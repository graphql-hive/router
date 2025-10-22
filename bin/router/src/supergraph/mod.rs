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
        } => Ok(SupergraphFileLoader::new(path, *poll_interval)?),
        SupergraphSource::HiveConsole {
            endpoint,
            key,
            poll_interval,
            retry_policy,
            timeout,
        } => {
            let patched_endpoint = match endpoint.ends_with("/supergraph") {
                true => endpoint.to_string(),
                false => format!("{}/supergraph", endpoint),
            };

            Ok(SupergraphHiveConsoleLoader::new(
                patched_endpoint,
                key,
                *poll_interval,
                *timeout,
                retry_policy.into(),
            )?)
        }
    }
}
