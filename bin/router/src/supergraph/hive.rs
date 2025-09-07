use async_trait::async_trait;
use http::header::USER_AGENT;
use tracing::debug;

use crate::supergraph::base::{LoadSupergraphError, SupergraphLoader};

static USER_AGENT_VALUE: &str = "hive-router";
static TIMEOUT: u64 = 10;
static AUTH_HEADER_NAME: &str = "X-Hive-CDN-Key";

pub struct SupergraphHiveConsoleLoader {
    endpoint: String,
    key: String,
    current: Option<String>,
    http_client: reqwest::Client,
}

#[async_trait]
impl SupergraphLoader for SupergraphHiveConsoleLoader {
    async fn reload(&mut self) -> Result<(), LoadSupergraphError> {
        debug!(
            "Fetching supergraph from Hive Console CDN: '{}'",
            self.endpoint
        );

        let response = self
            .http_client
            .get(&self.endpoint)
            .header(AUTH_HEADER_NAME, &self.key)
            .header(USER_AGENT, USER_AGENT_VALUE)
            .timeout(std::time::Duration::from_secs(TIMEOUT))
            .send()
            .await?
            .error_for_status()?;

        let content = response
            .text()
            .await
            .map_err(LoadSupergraphError::NetworkError)?;

        self.current = Some(content);
        Ok(())
    }

    fn current(&self) -> Option<&str> {
        self.current.as_deref()
    }
}

impl SupergraphHiveConsoleLoader {
    pub async fn new(endpoint: &str, key: &str) -> Result<Box<Self>, LoadSupergraphError> {
        debug!(
            "Creating supergraph source from Hive Console CDN: '{}'",
            endpoint
        );

        Ok(Box::new(Self {
            endpoint: endpoint.to_string(),
            key: key.to_string(),
            current: None,
            http_client: reqwest::Client::new(),
        }))
    }
}
