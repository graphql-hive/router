use crate::telemetry::logging::targets;
use async_trait::async_trait;
use hive_console_sdk::supergraph_fetcher::{
    async_fetcher::SupergraphFetcherAsyncState, SupergraphFetcher, SupergraphFetcherError,
};
use std::time::Duration;
use tracing::{debug, error};

use crate::{
    consts::ROUTER_VERSION,
    supergraph::base::{LoadSupergraphError, ReloadSupergraphResult, SupergraphLoader},
};

#[derive(Debug, thiserror::Error)]
#[allow(clippy::enum_variant_names)]
pub enum HiveConsoleSupergraphError {
    #[error("Failed to read supergraph from network: {0}")]
    NetworkError(#[from] reqwest_middleware::Error),
    #[error("Failed to read supergraph from network: {0}")]
    NetworkResponseError(#[from] reqwest::Error),
    #[error("Failed to lock supergraph: {0}")]
    LockError(String),
    #[error("Failed to initialize the loader: {0}")]
    InitializationError(String),
    #[error("Invalid configuration: {0}")]
    InvalidConfiguration(String),
    #[error("Hive CDN endpoint is missing. Please provide it via 'HIVE_CDN_ENDPOINT' environment variable or under 'supergraph.endpoint' in the configuration.")]
    MissingHiveCDNEndpoint,
    #[error("Hive CDN key is missing. Please provide it via 'HIVE_CDN_KEY' environment variable or under 'supergraph.key' in the configuration.")]
    MissingHiveCDNKey,
}

pub struct SupergraphHiveConsoleLoader {
    fetcher: SupergraphFetcher<SupergraphFetcherAsyncState>,
    poll_interval: Duration,
}

impl From<SupergraphFetcherError> for HiveConsoleSupergraphError {
    fn from(err: SupergraphFetcherError) -> Self {
        match err {
            SupergraphFetcherError::Network(e) => HiveConsoleSupergraphError::NetworkError(e),
            SupergraphFetcherError::ResponseParse(e) => {
                HiveConsoleSupergraphError::NetworkResponseError(e)
            }
            SupergraphFetcherError::ETagRead(e) => {
                HiveConsoleSupergraphError::LockError(format!("Failed to read etag: {:?}", e))
            }
            SupergraphFetcherError::ETagWrite(e) => {
                HiveConsoleSupergraphError::LockError(format!("Failed to write etag: {:?}", e))
            }
            SupergraphFetcherError::HTTPClientCreation(e) => {
                HiveConsoleSupergraphError::InitializationError(e.to_string())
            }
            SupergraphFetcherError::InvalidKey(e) => {
                HiveConsoleSupergraphError::InvalidConfiguration(format!("Invalid CDN key: {}", e))
            }
            SupergraphFetcherError::MissingConfigurationOption(msg) => {
                HiveConsoleSupergraphError::InvalidConfiguration(msg)
            }
            SupergraphFetcherError::RejectedByCircuitBreaker => {
                HiveConsoleSupergraphError::NetworkError(reqwest_middleware::Error::Middleware(
                    anyhow::anyhow!("Request rejected by circuit breaker"),
                ))
            }
            SupergraphFetcherError::CircuitBreakerCreation(e) => {
                HiveConsoleSupergraphError::InitializationError(format!(
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
                error!(
                  target: targets::SUPERGRAPH,
                  error = ?err,
                  "Error fetching supergraph from Hive Console",
                );
                Err(HiveConsoleSupergraphError::from(err).into())
            }
            // If the supergraph has not changed, return Unchanged
            Ok(None) => Ok(ReloadSupergraphResult::Unchanged),
            // If there is a new supergraph SDL, return it
            Ok(Some(sdl)) => Ok(ReloadSupergraphResult::Changed { new_sdl: sdl }),
        }
    }

    fn reload_interval(&self) -> Option<std::time::Duration> {
        Some(self.poll_interval)
    }
}

impl SupergraphHiveConsoleLoader {
    pub fn try_new(
        endpoints: Vec<String>,
        key: &str,
        poll_interval: Duration,
        connect_timeout: Duration,
        request_timeout: Duration,
        accept_invalid_certs: bool,
        retry_count: u32,
    ) -> Result<Box<Self>, HiveConsoleSupergraphError> {
        debug!(
          target: targets::SUPERGRAPH,
          endpoints = ?endpoints,
          interval_ms = poll_interval.as_millis(),
          request_timeout_ms = request_timeout.as_millis(),
          "Creating supergraph source from Hive Console CDN",
        );

        let mut fetcher_builder = SupergraphFetcher::builder()
            .user_agent(format!("hive-router/{}", ROUTER_VERSION))
            .key(key.to_string())
            .accept_invalid_certs(accept_invalid_certs)
            .connect_timeout(connect_timeout)
            .request_timeout(request_timeout)
            .max_retries(retry_count);

        for url in endpoints {
            fetcher_builder = fetcher_builder.add_endpoint(url);
        }

        let fetcher = fetcher_builder.build_async()?;

        Ok(Box::new(SupergraphHiveConsoleLoader {
            fetcher,
            poll_interval,
        }))
    }
}
