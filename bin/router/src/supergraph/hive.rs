use async_trait::async_trait;
use hive_console_sdk::supergraph_fetcher::{
    async_::SupergraphFetcherAsyncState, SupergraphFetcher, SupergraphFetcherError,
};
use reqwest_retry::policies::ExponentialBackoff;
use std::time::Duration;
use tracing::{debug, error};

use crate::{
    consts::ROUTER_VERSION,
    supergraph::base::{LoadSupergraphError, ReloadSupergraphResult, SupergraphLoader},
};

pub struct SupergraphHiveConsoleLoader {
    fetcher: SupergraphFetcher<SupergraphFetcherAsyncState>,
    poll_interval: Duration,
}

impl From<SupergraphFetcherError> for LoadSupergraphError {
    fn from(err: SupergraphFetcherError) -> Self {
        match err {
            SupergraphFetcherError::HTTPClientCreation(e) => {
                LoadSupergraphError::InitializationError(e.to_string())
            }
            SupergraphFetcherError::Network(e) => LoadSupergraphError::NetworkError(e),
            SupergraphFetcherError::ResponseParse(e) => {
                LoadSupergraphError::NetworkResponseError(e)
            }
            SupergraphFetcherError::ETagRead(e) => {
                LoadSupergraphError::LockError(format!("ETag read error: {:?}", e))
            }
            SupergraphFetcherError::ETagWrite(e) => {
                LoadSupergraphError::LockError(format!("ETag write error: {:?}", e))
            }
            SupergraphFetcherError::InvalidKey(e) => {
                LoadSupergraphError::InvalidConfiguration(format!("Invalid CDN key: {}", e))
            }
            SupergraphFetcherError::MissingConfigurationOption(e) => {
                LoadSupergraphError::InvalidConfiguration(format!(
                    "Missing configuration option: {}",
                    e
                ))
            }
            SupergraphFetcherError::RejectedByCircuitBreaker => {
                LoadSupergraphError::RejectedByCircuitBreaker
            }
            SupergraphFetcherError::CircuitBreakerCreation(e) => {
                LoadSupergraphError::InitializationError(e.to_string())
            }
        }
    }
}

#[async_trait]
impl SupergraphLoader for SupergraphHiveConsoleLoader {
    async fn load(&self) -> Result<ReloadSupergraphResult, LoadSupergraphError> {
        let fetcher_result = self.fetcher.fetch_supergraph().await;
        match fetcher_result {
            // If there was an error fetching the supergraph, propagate it
            Err(err) => {
                error!("Error fetching supergraph from Hive Console: {}", err);
                Err(LoadSupergraphError::from(err))
            }
            // If the supergraph has not changed, return Unchanged
            Ok(None) => Ok(ReloadSupergraphResult::Unchanged),
            // If there is a new supergraph SDL, return it
            Ok(Some(sdl)) => Ok(ReloadSupergraphResult::Changed { new_sdl: sdl }),
        }
    }

    fn reload_interval(&self) -> Option<&std::time::Duration> {
        Some(&self.poll_interval)
    }
}

impl SupergraphHiveConsoleLoader {
    pub fn try_new(
        endpoint: String,
        key: &str,
        poll_interval: Duration,
        connect_timeout: Duration,
        request_timeout: Duration,
        accept_invalid_certs: bool,
        retry_policy: ExponentialBackoff,
    ) -> Result<Box<Self>, LoadSupergraphError> {
        debug!(
            "Creating supergraph source from Hive Console CDN: '{}' (poll interval: {}ms, request_timeout: {}ms)",
            endpoint,
            poll_interval.as_millis(),
            request_timeout.as_millis()
        );
        let fetcher = SupergraphFetcher::builder()
            .add_endpoint(endpoint)
            .key(key.into())
            .user_agent(format!("hive-router/{}", ROUTER_VERSION))
            .connect_timeout(connect_timeout)
            .request_timeout(request_timeout)
            .accept_invalid_certs(accept_invalid_certs)
            .retry_policy(retry_policy)
            .build_async()?;

        Ok(Box::new(SupergraphHiveConsoleLoader {
            fetcher,
            poll_interval,
        }))
    }
}
