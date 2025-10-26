use std::str::FromStr;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, JsonSchema, Default)]
#[serde(deny_unknown_fields)]
pub struct LoggingConfig {
    /// The level of logging to use.
    ///
    /// Can also be set via the `LOG_LEVEL` environment variable.
    #[serde(default)]
    pub level: LogLevel,

    /// The format of the log messages.
    ///
    /// Can also be set via the `LOG_FORMAT` environment variable.
    #[serde(default)]
    pub format: LogFormat,

    /// The filter to apply to log messages.
    ///
    /// Can also be set via the `LOG_FILTER` environment variable.
    #[serde(default)]
    pub filter: Option<String>,
}

impl LoggingConfig {
    pub fn env_filter_str(&self) -> &str {
        self.filter.as_deref().unwrap_or(self.level.as_str())
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl FromStr for LogLevel {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "trace" => Ok(LogLevel::Trace),
            "debug" => Ok(LogLevel::Debug),
            "info" => Ok(LogLevel::Info),
            "warn" => Ok(LogLevel::Warn),
            "error" => Ok(LogLevel::Error),
            _ => Err(format!("Invalid log level: {}", s)),
        }
    }
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

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub enum LogFormat {
    #[serde(rename = "pretty-tree")]
    PrettyTree,
    #[serde(rename = "pretty-compact")]
    PrettyCompact,
    #[serde(rename = "json")]
    Json,
}

impl LogFormat {
    pub fn as_str(&self) -> &'static str {
        match self {
            LogFormat::PrettyTree => "pretty-tree",
            LogFormat::PrettyCompact => "pretty-compact",
            LogFormat::Json => "json",
        }
    }
}

impl FromStr for LogFormat {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "pretty-tree" => Ok(LogFormat::PrettyTree),
            "pretty-compact" => Ok(LogFormat::PrettyCompact),
            "json" => Ok(LogFormat::Json),
            _ => Err(format!("Invalid log format: {}", s)),
        }
    }
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
