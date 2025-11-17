use async_trait::async_trait;
use hive_console_sdk::supergraph_fetcher::{
    SupergraphFetcher, SupergraphFetcherAsyncState, SupergraphFetcherError,
};
use std::time::Duration;

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
            SupergraphFetcherError::NetworkError(e) => LoadSupergraphError::NetworkError(e),
            SupergraphFetcherError::NetworkResponseError(e) => {
                LoadSupergraphError::NetworkResponseError(e)
            }
            SupergraphFetcherError::Lock(e) => LoadSupergraphError::OtherError(e),
            SupergraphFetcherError::FetcherCreationError(e) => {
                LoadSupergraphError::OtherError(format!("Failed to create fetcher: {}", e))
            }
            SupergraphFetcherError::InvalidKey(e) => {
                LoadSupergraphError::OtherError(format!("Invalid CDN key: {}", e))
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
            Err(err) => Err(err.into()),
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
        retry_count: u32,
    ) -> Result<Box<Self>, LoadSupergraphError> {
        let fetcher = SupergraphFetcher::try_new_async(
            endpoint,
            key,
            format!("hive-router/{}", ROUTER_VERSION),
            connect_timeout,
            request_timeout,
            accept_invalid_certs,
            retry_count,
        )?;

        Ok(Box::new(SupergraphHiveConsoleLoader {
            fetcher,
            poll_interval,
        }))
    }
}
