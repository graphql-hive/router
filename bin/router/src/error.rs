use hive_router_config::RouterConfigError;

use crate::{jwt::jwks_manager::JwksSourceError, pipeline::usage_reporting::UsageReportingError, schema_state::SupergraphManagerError, shared_state::SharedStateError};

#[derive(Debug, thiserror::Error)]
pub enum RouterEntrypointError {
    #[error(transparent)]
    ConfigError(#[from] RouterConfigError),
    #[error("Failed to initialize usage reporting: {0}")]
    UsageReportingInitError(#[from] UsageReportingError),
    #[error("Failed to initialize the application state: {0}")]
    SharedStateError(#[from] SharedStateError),
    #[error("Failed to bind the HTTP server: {0}")]
    HttpServerBindError(std::io::Error),
    #[error("HTTP Server failed to run: {0}")]
    HttpServerRunError(std::io::Error),
    #[error("Failed to initialize supergraph manager: {0}")]
    SupergraphManagerError(#[from] SupergraphManagerError),
    #[error("Failed to initialize JWT: {0}")]
    JwtAuthInitError(#[from] JwksSourceError),
}