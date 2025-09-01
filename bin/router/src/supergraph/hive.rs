use async_trait::async_trait;
use http::{
    header::{ETAG, IF_NONE_MATCH, USER_AGENT},
    HeaderValue, StatusCode,
};
use reqwest_middleware::{ClientBuilder, ClientWithMiddleware};
use reqwest_retry::RetryTransientMiddleware;
use retry_policies::policies::ExponentialBackoff;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{debug, error};

use crate::supergraph::base::{LoadSupergraphError, ReloadSupergraphResult, SupergraphLoader};

static USER_AGENT_VALUE: &str = "hive-router/{}";
static AUTH_HEADER_NAME: &str = "x-hive-cdn-key";

pub struct SupergraphHiveConsoleLoader {
    endpoint: String,
    key: String,
    http_client: ClientWithMiddleware,
    poll_interval: Duration,
    timeout: Duration,
    last_etag: RwLock<Option<HeaderValue>>,
}

#[async_trait]
impl SupergraphLoader for SupergraphHiveConsoleLoader {
    async fn load(&self) -> Result<ReloadSupergraphResult, LoadSupergraphError> {
        debug!(
            "Fetching supergraph from Hive Console CDN: '{}'",
            self.endpoint,
        );

        let mut req = self
            .http_client
            .get(&self.endpoint)
            .header(AUTH_HEADER_NAME, &self.key)
            .header(USER_AGENT, USER_AGENT_VALUE)
            .timeout(self.timeout);

        let mut etag_used = false;

        match self.last_etag.try_read() {
            Ok(lock_guard) => {
                if let Some(etag) = lock_guard.as_ref() {
                    req = req.header(IF_NONE_MATCH, etag);
                    etag_used = true;
                }
            }
            Err(e) => {
                error!("Failed to read etag record: {:?}", e);
            }
        };

        let response = req.send().await?.error_for_status()?;

        if etag_used && response.status() == StatusCode::NOT_MODIFIED {
            Ok(ReloadSupergraphResult::Unchanged)
        } else {
            if let Some(new_etag) = response.headers().get(ETAG) {
                match self.last_etag.try_write() {
                    Ok(mut v) => {
                        debug!("saving etag record: {:?}", new_etag);
                        *v = Some(new_etag.clone());
                    }
                    Err(e) => {
                        error!("Failed to save etag record: {:?}", e);
                    }
                }
            }

            let content = response
                .text()
                .await
                .map_err(LoadSupergraphError::NetworkResponseError)?;

            Ok(ReloadSupergraphResult::Changed { new_sdl: content })
        }
    }

    fn reload_interval(&self) -> Option<&std::time::Duration> {
        Some(&self.poll_interval)
    }
}

impl SupergraphHiveConsoleLoader {
    pub fn new(
        endpoint: &str,
        key: &str,
        poll_interval: Duration,
        timeout: Duration,
        retry_policy: ExponentialBackoff,
    ) -> Result<Box<Self>, LoadSupergraphError> {
        debug!(
            "Creating supergraph source from Hive Console CDN: '{}' (poll interval: {}ms, timeout: {}ms)",
            endpoint,
            poll_interval.as_millis(),
            timeout.as_millis()
        );

        let client = ClientBuilder::new(reqwest::Client::new())
            .with(RetryTransientMiddleware::new_with_policy(retry_policy))
            .build();

        Ok(Box::new(Self {
            endpoint: endpoint.to_string(),
            key: key.to_string(),
            http_client: client,
            poll_interval,
            timeout,
            last_etag: RwLock::new(None),
        }))
    }
}
