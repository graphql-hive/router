use hive_router_config::jwt_auth::{JwksProviderSourceConfig, JwtAuthConfig};
use sonic_rs::from_str;
use std::sync::{Arc, RwLock};
use tokio::fs::read_to_string;

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
}

#[derive(Debug)]
pub struct JwksSource {
    config: JwksProviderSourceConfig,
    jwk: RwLock<Option<Arc<JwkSet>>>,
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
                tracing::debug!("loading jwks from a remote source: {}", url);

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
                tracing::debug!("loading jwks from a file source: {}", file.0);

                let file_contents = read_to_string(&file.0)
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

    pub fn should_prefetch(&self) -> bool {
        match &self.config {
            JwksProviderSourceConfig::Remote { prefetch, .. } => match prefetch {
                Some(prefetch) => *prefetch,
                None => false,
            },
            JwksProviderSourceConfig::File { .. } => true,
        }
    }

    pub async fn get_jwk_set(&self) -> Result<Arc<JwkSet>, JwksSourceError> {
        if let Ok(jwk) = self.jwk.try_read() {
            if let Some(jwk) = jwk.as_ref() {
                return Ok(jwk.clone());
            }
        }

        Err(JwksSourceError::FailedToAcquireJwk)
    }
}

// #[async_trait::async_trait]
// impl BackgroundTask for JwksProvidersManager {
//     fn id(&self) -> &str {
//         "jwt_auth"
//     }

//     async fn run(&self, token: CancellationToken) {
//         // let mut interval = tokio::time::interval(Duration::from_secs(3));
//         // loop {
//         //     tokio::select! {
//         //         _ = interval.tick() => { println!("running"); }
//         //         _ = token.cancelled() => { println!("Shutting down."); return; }
//         //     }
//         // }
//     }
// }
