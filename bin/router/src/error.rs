use hive_router_config::RouterConfigError;

use crate::{
    jwt::jwks_manager::JwksSourceError, pipeline::usage_reporting::UsageReportingError,
    schema_state::SupergraphManagerError, shared_state::SharedStateError,
};

#[derive(Debug, thiserror::Error)]
pub enum RouterInitError {
    #[error(transparent)]
    RouterConfigError(#[from] RouterConfigError),
    #[error(transparent)]
    SupergraphManagerError(#[from] SupergraphManagerError),
    #[error("Failed to bind HTTP server to address: {0}. Error: {1}")]
    HttpServerBindError(String, std::io::Error),
    #[error("Failed to start HTTP server: {0}")]
    HttpServerStartError(std::io::Error),
    #[error(transparent)]
    JwksSourceError(#[from] JwksSourceError),
    #[error("Usage Reporting - {0}")]
    UsageReportingError(#[from] UsageReportingError),
    #[error(transparent)]
    SharedStateError(#[from] SharedStateError),
}
