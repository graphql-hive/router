pub mod service;
pub mod shared;

use crate::log::service::ServiceLoggingConfig;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[derive(Default, Clone)]
pub struct LoggingConfig {
    #[serde(default)]
    pub service: ServiceLoggingConfig,
}
