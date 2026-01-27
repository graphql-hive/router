use async_trait::async_trait;
use hive_console_sdk::supergraph_fetcher::{
    async_fetcher::SupergraphFetcherAsyncState, SupergraphFetcher, SupergraphFetcherError,
};
use hive_router_config::supergraph::HiveConsoleCdnEndpoint;
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
            SupergraphFetcherError::Network(e) => LoadSupergraphError::NetworkError(e),
            SupergraphFetcherError::ResponseParse(e) => {
                LoadSupergraphError::NetworkResponseError(e)
            }
            SupergraphFetcherError::ETagRead(e) => {
                LoadSupergraphError::LockError(format!("Failed to read etag: {:?}", e))
            }
            SupergraphFetcherError::ETagWrite(e) => {
                LoadSupergraphError::LockError(format!("Failed to write etag: {:?}", e))
            }
            SupergraphFetcherError::HTTPClientCreation(e) => {
                LoadSupergraphError::InitializationError(e.to_string())
            }
            SupergraphFetcherError::InvalidKey(e) => {
                LoadSupergraphError::InvalidConfiguration(format!("Invalid CDN key: {}", e))
            }
            SupergraphFetcherError::MissingConfigurationOption(msg) => {
                LoadSupergraphError::InvalidConfiguration(msg)
            }
            SupergraphFetcherError::RejectedByCircuitBreaker => {
                LoadSupergraphError::NetworkError(reqwest_middleware::Error::Middleware(
                    anyhow::anyhow!("Request rejected by circuit breaker"),
                ))
            }
            SupergraphFetcherError::CircuitBreakerCreation(e) => {
                LoadSupergraphError::InitializationError(format!(
                    "Circuit breaker creation failed: {}",
                    e
                ))
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
        endpoint: HiveConsoleCdnEndpoint,
        key: &str,
        poll_interval: Duration,
        connect_timeout: Duration,
        request_timeout: Duration,
        accept_invalid_certs: bool,
        retry_count: u32,
    ) -> Result<Box<Self>, LoadSupergraphError> {
        debug!(
            "Creating supergraph source from Hive Console CDN: '{:#?}' (poll interval: {}ms, request_timeout: {}ms)",
            endpoint,
            poll_interval.as_millis(),
            request_timeout.as_millis()
        );
        let mut fetcher_builder = SupergraphFetcher::builder()
            .user_agent(format!("hive-router/{}", ROUTER_VERSION))
            .key(key.to_string())
            .accept_invalid_certs(accept_invalid_certs)
            .connect_timeout(connect_timeout)
            .request_timeout(request_timeout)
            .max_retries(retry_count);

        match endpoint {
            HiveConsoleCdnEndpoint::Single(url) => {
                fetcher_builder = fetcher_builder.add_endpoint(url);
            }
            HiveConsoleCdnEndpoint::Multiple(urls) => {
                for url in urls {
                    fetcher_builder = fetcher_builder.add_endpoint(url);
                }
            }
        }

        let fetcher = fetcher_builder.build_async()?;

        Ok(Box::new(SupergraphHiveConsoleLoader {
            fetcher,
            poll_interval,
        }))
    }
}
