#[derive(Debug, thiserror::Error)]
pub enum TelemetryError {
    /// Internal error
    #[error("internal error: {0}")]
    Internal(String),
    #[error("unable to configure traces exporter: {0}")]
    TracesExporterSetup(String),
}

impl From<String> for TelemetryError {
    fn from(s: String) -> Self {
        TelemetryError::Internal(s)
    }
}

impl From<&str> for TelemetryError {
    fn from(s: &str) -> Self {
        TelemetryError::Internal(s.to_string())
    }
}
