use crate::background_tasks::{BackgroundTask, BackgroundTasksManager};
use crate::config::jwt_auth::{JwksProviderSourceConfig, JwtAuthConfig};
use crate::telemetry::logging::targets;
use sonic_rs::from_str;
use std::sync::{Arc, RwLock};
use tokio::fs::read_to_string;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info};

use jsonwebtoken::jwk::JwkSet;

pub struct JwksManager {
    sources: Vec<Arc<JwksSource>>,
}

impl JwksManager {
    pub fn from_config(config: &JwtAuthConfig) -> Self {
        let sources = config
            .jwks_providers
            .iter()
            .map(|config| Arc::new(JwksSource::new(config.clone())))
            .collect();

        JwksManager { sources }
    }

    pub fn all(&self) -> Vec<Arc<JwkSet>> {
        self.sources
            .iter()
            .filter_map(|v| match v.get_jwk_set() {
                Ok(set) => Some(set),
                Err(err) => {
                    error!(target: targets::JWT, error = ?err, "failed to use jwt set, ignoring this set");

                    None
                }
            })
            .collect()
    }

    pub async fn prefetch_sources(&self) -> Result<(), JwksSourceError> {
        for source in &self.sources {
            if source.should_prefetch() {
                match source.load_and_store_jwks().await {
                    Ok(_) => {}
                    Err(err) => return Err(err),
                }
            }
        }

        Ok(())
    }

    pub fn register_background_tasks(&self, background_tasks_mgr: &mut BackgroundTasksManager) {
        for source in &self.sources {
            if source.should_poll_in_background() {
                background_tasks_mgr.register_task(JwksSourceTask(source.clone()));
            }
        }
    }
}

#[derive(Debug)]
pub struct JwksSource {
    config: JwksProviderSourceConfig,
    jwk: RwLock<Option<Arc<JwkSet>>>,
}

struct JwksSourceTask(Arc<JwksSource>);

#[async_trait::async_trait]
impl BackgroundTask for JwksSourceTask {
    fn id(&self) -> &str {
        "jwt_auth_jwks"
    }

    async fn run(&self, token: CancellationToken) {
        if let JwksProviderSourceConfig::Remote {
            polling_interval: Some(interval),
            url,
            ..
        } = &self.0.config
        {
            info!(
                target: targets::JWT,
                url = ?url,
                "starting remote jwks polling for source",
            );
            let mut tokio_interval = tokio::time::interval(*interval);

            loop {
                tokio::select! {
                    _ = tokio_interval.tick() => { match self.0.load_and_store_jwks().await {
                        Ok(_) => {}
                        Err(err) => {
                            error!(target: targets::JWT, error = ?err, url = ?url, "failed to load remote jwks");
                        }
                    } }
                    _ = token.cancelled() => { info!(target: targets::JWT, "jwks source shutting down."); return; }
                }
            }
        }
    }
}

#[derive(thiserror::Error, Debug)]
pub enum JwksSourceError {
    #[error("failed to load remote jwks: {0}")]
    RemoteJwksNetworkError(reqwest::Error),
    #[error("failed to load file jwks: {0}")]
    FileJwksNetworkError(std::io::Error),
    #[error("failed to parse jwks json file: {0}")]
    JwksContentInvalidStructure(sonic_rs::Error),
    #[error("failed to acquire jwks handle")]
    FailedToAcquireJwk,
}

impl JwksSource {
    async fn load_and_store_jwks(&self) -> Result<&Self, JwksSourceError> {
        let jwks_str = match &self.config {
            JwksProviderSourceConfig::Remote { url, headers, .. } => {
                let client = reqwest::Client::new();
                debug!(target: targets::JWT, url = ?url, "loading jwks from a remote source");

                // `headers()` carries `Host` through as an override of the
                // authority derived from `url`, without changing which address
                // the request is sent to. That is what makes an internal URL
                // usable against a virtual-host-routed issuer. An empty map is
                // a no-op, so this is unconditional.
                let response_text = client
                    .get(url)
                    .headers(headers.clone())
                    .send()
                    .await
                    .map_err(JwksSourceError::RemoteJwksNetworkError)?
                    .error_for_status()
                    .map_err(JwksSourceError::RemoteJwksNetworkError)?
                    .text()
                    .await
                    .map_err(JwksSourceError::RemoteJwksNetworkError)?;

                response_text
            }
            JwksProviderSourceConfig::File { file, .. } => {
                debug!(target: targets::JWT, path = ?file.absolute, "loading jwks from a file source");

                let file_contents = read_to_string(&file.absolute)
                    .await
                    .map_err(JwksSourceError::FileJwksNetworkError)?;

                file_contents
            }
        };

        let new_jwk = Arc::new(
            from_str::<JwkSet>(&jwks_str).map_err(JwksSourceError::JwksContentInvalidStructure)?,
        );

        if let Ok(mut w_jwk) = self.jwk.write() {
            *w_jwk = Some(new_jwk);
        }

        Ok(self)
    }

    pub fn new(config: JwksProviderSourceConfig) -> Self {
        Self {
            config,
            jwk: RwLock::new(None),
        }
    }

    pub fn should_poll_in_background(&self) -> bool {
        match &self.config {
            JwksProviderSourceConfig::Remote { .. } => true,
            JwksProviderSourceConfig::File { .. } => false,
        }
    }

    pub fn should_prefetch(&self) -> bool {
        match &self.config {
            JwksProviderSourceConfig::Remote { prefetch, .. } => match prefetch {
                Some(prefetch) => *prefetch,
                None => false,
            },
            JwksProviderSourceConfig::File { .. } => true,
        }
    }

    pub fn get_jwk_set(&self) -> Result<Arc<JwkSet>, JwksSourceError> {
        if let Ok(jwk) = self.jwk.try_read() {
            if let Some(jwk) = jwk.as_ref() {
                return Ok(jwk.clone());
            }
        }

        Err(JwksSourceError::FailedToAcquireJwk)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::{HeaderMap, HeaderName, HeaderValue};

    const JWKS_BODY: &str = r#"{"keys":[]}"#;

    fn remote_source(url: String, headers: HeaderMap) -> JwksSource {
        JwksSource {
            config: JwksProviderSourceConfig::Remote {
                url,
                polling_interval: None,
                prefetch: Some(false),
                headers,
            },
            jwk: RwLock::new(None),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn remote_jwks_sends_configured_headers() {
        crate::init_rustls_crypto_provider();
        let mut server = mockito::Server::new_async().await;

        // `Host` is the reason this feature exists: it must reach the server as
        // an override of the authority derived from the URL, while the request
        // still goes to the URL's address.
        let mock = server
            .mock("GET", "/jwks.json")
            .match_header("host", "auth.example.com")
            .match_header("x-api-key", "secret")
            .expect(1)
            .with_status(200)
            .with_body(JWKS_BODY)
            .create_async()
            .await;

        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("host"),
            HeaderValue::from_static("auth.example.com"),
        );
        headers.insert(
            HeaderName::from_static("x-api-key"),
            HeaderValue::from_static("secret"),
        );

        let source = remote_source(format!("{}/jwks.json", server.url()), headers);
        source
            .load_and_store_jwks()
            .await
            .expect("jwks should load");

        mock.assert_async().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn remote_jwks_without_headers_is_unchanged() {
        crate::init_rustls_crypto_provider();
        let mut server = mockito::Server::new_async().await;

        let mock = server
            .mock("GET", "/jwks.json")
            .expect(1)
            .with_status(200)
            .with_body(JWKS_BODY)
            .create_async()
            .await;

        let source = remote_source(format!("{}/jwks.json", server.url()), HeaderMap::new());
        source
            .load_and_store_jwks()
            .await
            .expect("jwks should load");

        mock.assert_async().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn remote_jwks_error_status_is_reported_as_network_error() {
        crate::init_rustls_crypto_provider();
        let mut server = mockito::Server::new_async().await;

        let mock = server
            .mock("GET", "/jwks.json")
            .expect(1)
            .with_status(401)
            .with_body("unauthorized")
            .create_async()
            .await;

        let source = remote_source(format!("{}/jwks.json", server.url()), HeaderMap::new());
        let err = source
            .load_and_store_jwks()
            .await
            .expect_err("401 response should not be parsed as jwks");

        assert!(matches!(err, JwksSourceError::RemoteJwksNetworkError(_)));

        mock.assert_async().await;
    }
}
