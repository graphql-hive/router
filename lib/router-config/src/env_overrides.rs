use config::{builder::BuilderState, ConfigBuilder, ConfigError};
use envconfig::Envconfig;
use tracing::debug;

use crate::{log::{LogFormat, LogLevel}, non_empty_string::NoneIfEmpty};

#[derive(Envconfig)]
pub struct EnvVarOverrides {
    // Logger overrides
    #[envconfig(from = "LOG_LEVEL")]
    pub log_level: Option<LogLevel>,
    #[envconfig(from = "LOG_FORMAT")]
    pub log_format: Option<LogFormat>,
    #[envconfig(from = "LOG_FILTER")]
    pub log_filter: Option<String>,

    // GraphiQL overrides
    #[envconfig(from = "GRAPHIQL_ENABLED")]
    pub graphiql_enabled: Option<bool>,

    // HTTP overrides
    #[envconfig(from = "PORT")]
    pub http_port: Option<u64>,
    #[envconfig(from = "HOST")]
    pub http_host: Option<String>,

    // Supergraph overrides
    #[envconfig(from = "SUPERGRAPH_FILE_PATH")]
    pub supergraph_file_path: Option<String>,
    #[envconfig(from = "HIVE_CDN_ENDPOINT")]
    pub hive_console_cdn_endpoint: Option<String>,
    #[envconfig(from = "HIVE_CDN_KEY")]
    pub hive_console_cdn_key: Option<String>,
    #[envconfig(from = "HIVE_CDN_POLL_INTERVAL")]
    pub hive_console_cdn_poll_interval: Option<String>,
    #[envconfig(from = "HIVE_ACCESS_TOKEN")]
    pub hive_access_token: Option<String>,
    #[envconfig(from = "HIVE_TARGET")]
    pub hive_target: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum EnvVarOverridesError {
    #[error("Failed to override configuration: {0}")]
    FailedToOverrideConfig(#[from] ConfigError),
    #[error("Cannot override supergraph source due to conflict: SUPERGRAPH_FILE_PATH and HIVE_CDN_ENDPOINT cannot be used together")]
    ConflictingSupergraphSource,
    #[error("Missing required environment variable: {0}")]
    MissingRequiredEnvVar(&'static str),
}

impl EnvVarOverrides {
    pub fn apply_overrides<T: BuilderState>(
        mut self,
        mut config: ConfigBuilder<T>,
    ) -> Result<ConfigBuilder<T>, EnvVarOverridesError> {
        if let Some(log_level) = self.log_level.take() {
            debug!("[config-override] 'log.level' = {:?}", log_level);
            config = config.set_override("log.level", log_level.as_str())?;
        }
        if let Some(log_format) = self.log_format.take() {
            debug!("[config-override] 'log.format' = {:?}", log_format);
            config = config.set_override("log.format", log_format.as_str())?;
        }
        if let Some(log_filter) = self.log_filter.take().none_if_empty() {
            debug!("[config-override] 'log.filter' = {:?}", log_filter);
            config = config.set_override("log.filter", log_filter)?;
        }

        if let Some(http_port) = self.http_port.take() {
            debug!("[config-override] 'http.port' = {}", http_port);
            config = config.set_override("http.port", http_port)?;
        }

        if let Some(http_host) = self.http_host.take().none_if_empty() {
            debug!("[config-override] 'http.host' = {}", http_host);
            config = config.set_override("http.host", http_host)?;
        }

        let mut supergraph_file_path = self.supergraph_file_path.none_if_empty();
        let mut hive_console_cdn_endpoint = self.hive_console_cdn_endpoint.none_if_empty();

        if supergraph_file_path.is_some() && hive_console_cdn_endpoint.is_some() {
            return Err(EnvVarOverridesError::ConflictingSupergraphSource);
        }

        if let Some(supergraph_file_path) = supergraph_file_path.take() {
            config = config.set_override("supergraph.source", "file")?;
            config = config.set_override("supergraph.path", supergraph_file_path)?;
        }

        if let Some(hive_console_cdn_endpoint) = hive_console_cdn_endpoint.take().none_if_empty() {
            config = config.set_override("supergraph.source", "hive")?;
            config = config.set_override("supergraph.endpoint", hive_console_cdn_endpoint)?;

            if let Some(hive_console_cdn_key) = self.hive_console_cdn_key.take().none_if_empty() {
                config = config.set_override("supergraph.key", hive_console_cdn_key)?;
            } else {
                return Err(EnvVarOverridesError::MissingRequiredEnvVar("HIVE_CDN_KEY"));
            }

            if let Some(hive_console_cdn_poll_interval) = self.hive_console_cdn_poll_interval.take().none_if_empty()
            {
                config = config
                    .set_override("supergraph.poll_interval", hive_console_cdn_poll_interval)?;
            }
        }

        if let Some(hive_access_token) = self.hive_access_token.take().none_if_empty() {
            config = config.set_override("usage_reporting.access_token", hive_access_token)?;
            if let Some(hive_target) = self.hive_target.take() {
                config = config.set_override("usage_reporting.target_id", hive_target)?;
            }
            config = config.set_override("usage_reporting.enabled", true)?;
        }

        // GraphiQL overrides
        if let Some(graphiql_enabled) = self.graphiql_enabled.take() {
            config = config.set_override("graphiql.enabled", graphiql_enabled)?;
        }

        Ok(config)
    }
}
