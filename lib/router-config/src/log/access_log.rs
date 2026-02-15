use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AccessLogLoggingConfig {
    exporters: Vec<AccessLogExporterConfig>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields, tag = "kind")]
pub enum AccessLogExporterConfig {
    File(AccessLogFileExporterConfig),
    // Stdout(AccessLogStdoutExporterConfig),
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AccessLogFileExporterConfig {
    path: String,
    attributes: AccessLogAttributes,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AccessLogAttributes {
    pub timestamp: bool,
    pub request_id: bool,
    pub status_code: bool,
    pub duration_ms: bool,
}
