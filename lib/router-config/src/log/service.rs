use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    log::shared::{default_log_internals, LogFormat, LogLevel},
    primitives::http_header::HttpHeaderName,
};

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
    #[serde(default)]
    pub http: HttpLogFieldsConfig,
    #[serde(default)]
    pub graphql: GraphQLLogFieldsConfig,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, Default)]
#[serde(deny_unknown_fields)]
pub struct HttpLogFieldsConfig {
    #[serde(default)]
    pub request: HttpRequestLogFieldsConfig,
    #[serde(default)]
    pub response: HttpResponseLogFieldsConfig,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, Default)]
#[serde(deny_unknown_fields)]
pub struct GraphQLLogFieldsConfig {
    #[serde(default)]
    pub request: GraphQLRequestLogFieldsConfig,
    #[serde(default)]
    pub response: GraphQLResponseLogFieldsConfig,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct HttpRequestLogFieldsConfig {
    pub method: bool,
    pub path: bool,
    pub query_string: bool,
    pub headers: Vec<HttpHeaderName>,
}

impl Default for HttpRequestLogFieldsConfig {
    fn default() -> Self {
        HttpRequestLogFieldsConfig {
            method: true,
            path: true,
            query_string: false,
            headers: vec!["accept".into(), "user-agent".into()],
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct HttpResponseLogFieldsConfig {
    pub status_code: bool,
    pub duration_ms: bool,
    pub headers: Vec<HttpHeaderName>,
    pub payload_bytes: bool,
}

impl Default for HttpResponseLogFieldsConfig {
    fn default() -> Self {
        HttpResponseLogFieldsConfig {
            status_code: true,
            duration_ms: true,
            headers: vec![],
            payload_bytes: false,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct GraphQLRequestLogFieldsConfig {
    pub body_size_bytes: bool,
    pub client_name: bool,
    pub client_version: bool,
    pub operation: bool,
    pub operation_name: bool,
    pub variables: bool,
    pub extensions: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct GraphQLResponseLogFieldsConfig {}

impl Default for GraphQLRequestLogFieldsConfig {
    fn default() -> Self {
        GraphQLRequestLogFieldsConfig {
            client_name: true,
            client_version: true,
            operation_name: true,
            body_size_bytes: false,
            operation: false,
            variables: false,
            extensions: false,
        }
    }
}

impl Default for GraphQLResponseLogFieldsConfig {
    fn default() -> Self {
        GraphQLResponseLogFieldsConfig {}
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
