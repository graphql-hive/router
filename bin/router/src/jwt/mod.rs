pub mod context;
pub mod errors;
pub mod jwks_manager;

use std::sync::Arc;

use hive_router_config::jwt_auth::JwtAuthConfig;
use ntex::web::HttpRequest;

use crate::{
    background_tasks::BackgroundTasksManager,
    jwt::{
        errors::JwtError,
        jwks_manager::{JwksManager, JwksSourceError},
    },
};

pub struct JwtAuthRuntime {
    config: JwtAuthConfig,
    jwks: JwksManager,
}

impl JwtAuthRuntime {
    pub async fn init(
        background_tasks_mgr: &mut BackgroundTasksManager,
        config: &JwtAuthConfig,
    ) -> Result<Self, JwksSourceError> {
        let jwks = JwksManager::from_config(config);
        // If any of the sources needs to be prefetched (loaded when the server starts), then we'll
        // try to load it now, and fail if it fails.
        jwks.prefetch_sources().await?;

        let instance = JwtAuthRuntime {
            config: config.clone(),
            jwks,
        };

        Ok(instance)
    }

    pub fn validate_request(&self, request: &mut HttpRequest) -> Result<(), JwtError> {
        Ok(())
    }
}
