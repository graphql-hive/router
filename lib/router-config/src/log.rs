use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, JsonSchema, Default)]
pub struct LoggingConfig {
    #[serde(default)]
    pub level: LogLevel,
    #[serde(default)]
    pub format: LogFormat,
    #[serde(default)]
    pub filter: Option<String>,
}

impl LoggingConfig {
    pub fn env_filter_str(&self) -> &str {
        self.filter.as_deref().unwrap_or(self.level.as_str())
    }
}

#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl Default for LogLevel {
    #[cfg(debug_assertions)]
    fn default() -> Self {
        LogLevel::Debug
    }

    #[cfg(not(debug_assertions))]
    fn default() -> Self {
        LogLevel::Info
    }
}

impl LogLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            LogLevel::Trace => "trace",
            LogLevel::Debug => "debug",
            LogLevel::Info => "info",
            LogLevel::Warn => "warn",
            LogLevel::Error => "error",
        }
    }
}

#[derive(Deserialize, Serialize, JsonSchema)]
pub enum LogFormat {
    #[serde(rename = "pretty-tree")]
    PrettyTree,
    #[serde(rename = "pretty-compact")]
    PrettyCompact,
    #[serde(rename = "json")]
    Json,
}

impl Default for LogFormat {
    #[cfg(debug_assertions)]
    fn default() -> Self {
        LogFormat::PrettyCompact
    }

    #[cfg(not(debug_assertions))]
    fn default() -> Self {
        LogFormat::Json
    }
}
