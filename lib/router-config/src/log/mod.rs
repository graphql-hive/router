pub mod access_log;
pub mod service;
pub mod shared;

use crate::log::{
    access_log::AccessLogLoggingConfig,
    service::{ServiceLogExporter, ServiceLoggingConfig, StdoutExporterConfig},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LoggingConfig {
    pub service: ServiceLogging,
    pub access_log: Option<AccessLogLoggingConfig>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum ServiceLogging {
    Shortcut(StdoutExporterConfig),
    Advanced(ServiceLoggingConfig),
}

impl ServiceLogging {
    pub fn as_list(&self) -> ServiceLoggingConfig {
        match self {
            ServiceLogging::Advanced(c) => c.clone(),
            ServiceLogging::Shortcut(c) => ServiceLoggingConfig {
                exporters: vec![ServiceLogExporter::Stdout(c.clone())],
            },
        }
    }
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            service: ServiceLogging::Shortcut(StdoutExporterConfig::default()),
            access_log: None,
        }
    }
}
