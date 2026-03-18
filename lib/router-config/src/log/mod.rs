pub mod access_log;
pub mod service;
pub mod shared;

use crate::log::{access_log::AccessLogLoggingConfig, service::ServiceLoggingConfig};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[derive(Default)]
pub struct LoggingConfig {
    #[serde(default)]
    pub service: ServiceLoggingConfig,
    #[serde(default)]
    pub access_log: Option<AccessLogLoggingConfig>,
}

