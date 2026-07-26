use std::str::FromStr;

use http::HeaderName;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tracing::level_filters::LevelFilter;

use crate::primitives::http_header::HttpHeaderName;

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

    /// The correlation configuration for the logger.
    ///
    /// This is used to configure the correlation Request-ID header and W3C trace propagation.
    #[serde(default)]
    pub correlation: CorrelationConfig,

    /// Whether to log internal crates events.
    ///
    /// This is useful for debugging purposes, but should be disabled in production.
    #[serde(default = "default_log_internals")]
    pub log_internals: bool,
}

fn default_log_internals() -> bool {
    false
}

impl LoggingConfig {
    pub fn env_filter_str(&self) -> &str {
        self.filter.as_deref().unwrap_or(self.level.as_str())
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    #[cfg(debug_assertions)]
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
            #[cfg(debug_assertions)]
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
            #[cfg(debug_assertions)]
            LogLevel::Trace => "trace",
            LogLevel::Debug => "debug",
            LogLevel::Info => "info",
            LogLevel::Warn => "warn",
            LogLevel::Error => "error",
        }
    }
}

impl From<&LogLevel> for LevelFilter {
    fn from(val: &LogLevel) -> Self {
        match val {
            #[cfg(debug_assertions)]
            LogLevel::Trace => LevelFilter::TRACE,
            LogLevel::Debug => LevelFilter::DEBUG,
            LogLevel::Info => LevelFilter::INFO,
            LogLevel::Warn => LevelFilter::WARN,
            LogLevel::Error => LevelFilter::ERROR,
        }
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
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

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CorrelationConfig {
    #[serde(default = "default_correlation_id_header")]
    pub id_header: HttpHeaderName,
    #[serde(default = "default_trace_propagation")]
    pub trace_propagation: bool,
}

fn default_correlation_id_header() -> HttpHeaderName {
    HttpHeaderName::from(HeaderName::from_static("x-request-id"))
}

fn default_trace_propagation() -> bool {
    true
}

impl Default for CorrelationConfig {
    fn default() -> Self {
        Self {
            id_header: default_correlation_id_header(),
            trace_propagation: default_trace_propagation(),
        }
    }
}
