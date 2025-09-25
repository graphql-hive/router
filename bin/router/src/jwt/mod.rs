pub mod context;
pub mod errors;
pub mod jwks_manager;

use std::sync::Arc;

use hive_router_config::jwt_auth::JwtAuthConfig;
use ntex::web::HttpRequest;

use crate::{
    background_tasks::BackgroundTasksManager,
    jwt::{errors::JwtError, jwks_manager::JwksProvidersManager},
};

pub struct JwtAuthRuntime {
    config: JwtAuthConfig,
    jwks_manager: Arc<JwksProvidersManager>,
}

impl JwtAuthRuntime {
    pub fn init(background_tasks_mgr: &mut BackgroundTasksManager, config: &JwtAuthConfig) -> Self {
        let jwks_manager = Arc::new(JwksProvidersManager::from_config(config));
        let instance = JwtAuthRuntime {
            config: config.clone(),
            jwks_manager: jwks_manager.clone(),
        };

        background_tasks_mgr.register_task(jwks_manager);

        instance
    }

    pub fn validate_request(&self, request: &mut HttpRequest) -> Result<(), JwtError> {
        Ok(())
    }
}
