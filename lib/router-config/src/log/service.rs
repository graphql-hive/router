use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::log::shared::{default_log_internals, LogFormat, LogLevel};

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ServiceLoggingConfig {
    pub log_fields: LogFieldsConfig,
    pub exporters: Vec<ServiceLogExporter>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LogFieldsConfig {
    pub http: HttpLogFieldsConfig,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HttpLogFieldsConfig {
    pub request: HttpRequestLogFieldsConfig,
    pub response: HttpResponseLogFieldsConfig,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HttpRequestLogFieldsConfig {
    pub method: bool,
    pub path: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HttpResponseLogFieldsConfig {
    pub status_code: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields, tag = "kind")]
pub struct StdoutExporterConfig {
    #[serde(default = "LogLevel::default")]
    pub level: LogLevel,
    #[serde(default = "LogFormat::default")]
    pub format: LogFormat,
    #[serde(default = "default_log_internals")]
    pub log_internals: bool,
}

impl Default for StdoutExporterConfig {
    fn default() -> Self {
        StdoutExporterConfig {
            format: LogFormat::default(),
            level: LogLevel::default(),
            log_internals: default_log_internals(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields, tag = "kind")]
pub struct FileExporterConfig {
    pub path: String,
    #[serde(default)]
    pub rolling: Option<FileRolling>,
    #[serde(default = "LogLevel::default")]
    pub level: LogLevel,
    #[serde(default = "LogFormat::default")]
    pub format: LogFormat,
    #[serde(default = "default_log_internals")]
    pub log_internals: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields, tag = "kind")]
pub enum ServiceLogExporter {
    #[serde(rename = "stdout")]
    Stdout(StdoutExporterConfig),
    #[serde(rename = "file")]
    File(FileExporterConfig),
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub enum FileRolling {
    #[serde(rename = "minutely")]
    Minutely,
    #[serde(rename = "hourly")]
    Hourly,
    #[serde(rename = "daily")]
    Daily,
}
