use std::str::FromStr;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tracing::level_filters::LevelFilter;

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

impl FromStr for LogLevel {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
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

pub fn default_log_internals() -> bool {
    false
}

impl LogLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            LogLevel::Debug => "debug",
            LogLevel::Info => "info",
            LogLevel::Warn => "warn",
            LogLevel::Error => "error",
        }
    }
}

impl Into<LevelFilter> for &LogLevel {
    fn into(self) -> LevelFilter {
        match self {
            LogLevel::Debug => LevelFilter::DEBUG,
            LogLevel::Info => LevelFilter::INFO,
            LogLevel::Warn => LevelFilter::WARN,
            LogLevel::Error => LevelFilter::ERROR,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub enum LogFormat {
    #[serde(rename = "text")]
    Text,
    #[serde(rename = "json")]
    Json,
}

impl LogFormat {
    pub fn as_str(&self) -> &'static str {
        match self {
            LogFormat::Text => "text",
            LogFormat::Json => "json",
        }
    }
}

impl FromStr for LogFormat {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "text" => Ok(LogFormat::Text),
            "json" => Ok(LogFormat::Json),
            _ => Err(format!("Invalid log format: {}", s)),
        }
    }
}

impl Default for LogFormat {
    #[cfg(debug_assertions)]
    fn default() -> Self {
        LogFormat::Text
    }

    #[cfg(not(debug_assertions))]
    fn default() -> Self {
        LogFormat::Json
    }
}
