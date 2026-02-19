use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::log::shared::{default_log_internals, LogFormat, LogLevel};

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ServiceLoggingConfig {
    #[serde(default)]
    pub log_fields: LogFieldsConfig,
    pub exporters: Vec<ServiceLogExporter>,
}

impl Default for ServiceLoggingConfig {
    fn default() -> Self {
        Self {
            log_fields: LogFieldsConfig::default(),
            exporters: vec![ServiceLogExporter::Stdout(StdoutExporterConfig::default())],
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, Default)]
#[serde(deny_unknown_fields)]
pub struct LogFieldsConfig {
    #[serde(default = "HttpLogFieldsConfig::default")]
    pub http: HttpLogFieldsConfig,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, Default)]
#[serde(deny_unknown_fields)]
pub struct HttpLogFieldsConfig {
    #[serde(default = "HttpRequestLogFieldsConfig::default")]
    pub request: HttpRequestLogFieldsConfig,
    #[serde(default = "HttpResponseLogFieldsConfig::default")]
    pub response: HttpResponseLogFieldsConfig,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct HttpRequestLogFieldsConfig {
    pub method: bool,
    pub path: bool,
}

impl Default for HttpRequestLogFieldsConfig {
    fn default() -> Self {
        HttpRequestLogFieldsConfig {
            method: true,
            path: true,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct HttpResponseLogFieldsConfig {
    pub status_code: bool,
    pub duration_ms: bool,
}

impl Default for HttpResponseLogFieldsConfig {
    fn default() -> Self {
        HttpResponseLogFieldsConfig {
            status_code: true,
            duration_ms: true,
        }
    }
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
