use hive_router_config::jwt_auth::{JwksProviderSourceConfig, JwtAuthConfig};
use hive_router_internal::background_tasks::{BackgroundTask, BackgroundTasksManager};
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
                    error!("Failed to use JWK set: {}, ignoring", err);

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
            ..
        } = &self.0.config
        {
            debug!(
                "Starting remote jwks polling for source: {:?}",
                self.0.config
            );
            let mut tokio_interval = tokio::time::interval(*interval);

            loop {
                tokio::select! {
                    _ = tokio_interval.tick() => { match self.0.load_and_store_jwks().await {
                        Ok(_) => {}
                        Err(err) => {
                            error!("Failed to load remote jwks: {}", err);
                        }
                    } }
                    _ = token.cancelled() => { info!("Jwks source shutting down."); return; }
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
            JwksProviderSourceConfig::Remote { url, .. } => {
                let client = reqwest::Client::new();
                debug!("loading jwks from a remote source: {}", url);

                let response_text = client
                    .get(url)
                    .send()
                    .await
                    .map_err(JwksSourceError::RemoteJwksNetworkError)?
                    .text()
                    .await
                    .map_err(JwksSourceError::RemoteJwksNetworkError)?;

                response_text
            }
            JwksProviderSourceConfig::File { file, .. } => {
                debug!("loading jwks from a file source: {}", file.absolute);

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
