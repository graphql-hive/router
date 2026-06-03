use config::{builder::BuilderState, ConfigBuilder, ConfigError};
use envconfig::Envconfig;
use tracing::debug;

use crate::log::{LogFormat, LogLevel};

#[derive(Default, Envconfig)]
pub struct EnvVarOverrides {
    // Logger overrides
    #[envconfig(from = "LOG_LEVEL")]
    pub log_level: Option<LogLevel>,
    #[envconfig(from = "LOG_FORMAT")]
    pub log_format: Option<LogFormat>,
    #[envconfig(from = "LOG_FILTER")]
    pub log_filter: Option<String>,

    // Laboratory overrides
    #[envconfig(from = "LABORATORY_ENABLED")]
    pub laboratory_enabled: Option<bool>,

    // WebSocket overrides
    #[envconfig(from = "WEBSOCKET_ENABLED")]
    pub websocket_enabled: Option<bool>,

    // Subscriptions overrides
    #[envconfig(from = "SUBSCRIPTIONS_ENABLED")]
    pub subscriptions_enabled: Option<bool>,

    // HTTP overrides
    #[envconfig(from = "PORT")]
    pub http_port: Option<u64>,
    #[envconfig(from = "HOST")]
    pub http_host: Option<String>,
    #[envconfig(from = "ROUTER_HTTP_WORKERS")]
    pub http_workers: Option<usize>,

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
    #[envconfig(from = "HIVE_TRACING_ENABLED")]
    pub hive_tracing_enabled: Option<bool>,
    #[envconfig(from = "HIVE_USAGE_REPORTING_ENABLED")]
    pub hive_usage_reporting_enabled: Option<bool>,

    // Tracing overrides
    #[envconfig(from = "TELEMETRY_TRACING_SAMPLING_RATE")]
    pub tracing_sampling_rate: Option<f64>,
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
        if let Some(log_filter) = self.log_filter.take() {
            debug!("[config-override] 'log.filter' = {:?}", log_filter);
            config = config.set_override("log.filter", log_filter)?;
        }

        if let Some(http_port) = self.http_port.take() {
            debug!("[config-override] 'http.port' = {}", http_port);
            config = config.set_override("http.port", http_port)?;
        }

        if let Some(http_host) = self.http_host.take() {
            debug!("[config-override] 'http.host' = {}", http_host);
            config = config.set_override("http.host", http_host)?;
        }

        if let Some(http_workers) = self.http_workers.take() {
            debug!("[config-override] 'http.workers' = {}", http_workers);
            // cast to u64 because the `config` crate doesn't implement `Into<Value>` for `usize`;
            // the value is then deserialized into `Option<NonZeroUsize>`, which rejects `0`.
            config = config.set_override("http.workers", http_workers as u64)?;
        }

        if self.supergraph_file_path.is_some() && self.hive_console_cdn_endpoint.is_some() {
            return Err(EnvVarOverridesError::ConflictingSupergraphSource);
        }

        if let Some(supergraph_file_path) = self.supergraph_file_path.take() {
            config = config.set_override("supergraph.source", "file")?;
            config = config.set_override("supergraph.path", supergraph_file_path)?;
        }

        if let Some(hive_console_cdn_endpoint) = self.hive_console_cdn_endpoint.take() {
            config = config.set_override("supergraph.source", "hive")?;

            if hive_console_cdn_endpoint.contains(",") {
                let endpoints: Vec<String> = hive_console_cdn_endpoint
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .collect();
                config = config.set_override("supergraph.endpoint", endpoints)?;
            } else {
                config = config.set_override("supergraph.endpoint", hive_console_cdn_endpoint)?;
            }

            if let Some(hive_console_cdn_key) = self.hive_console_cdn_key.take() {
                config = config.set_override("supergraph.key", hive_console_cdn_key)?;
            } else {
                return Err(EnvVarOverridesError::MissingRequiredEnvVar("HIVE_CDN_KEY"));
            }

            if let Some(hive_console_cdn_poll_interval) = self.hive_console_cdn_poll_interval.take()
            {
                config = config
                    .set_override("supergraph.poll_interval", hive_console_cdn_poll_interval)?;
            }
        }

        if let Some(enabled) = self.hive_tracing_enabled.take() {
            config = config.set_override("telemetry.hive.tracing.enabled", enabled)?;
        }

        if let Some(enabled) = self.hive_usage_reporting_enabled.take() {
            config = config.set_override("telemetry.hive.usage_reporting.enabled", enabled)?;
        }

        if let Some(hive_access_token) = self.hive_access_token.take() {
            config = config.set_override("telemetry.hive.token", hive_access_token)?;
        }

        if let Some(hive_target) = self.hive_target.take() {
            config = config.set_override("telemetry.hive.target", hive_target)?;
        }

        if let Some(tracing_sampling_rate) = self.tracing_sampling_rate.take() {
            debug!(
                "[config-override] 'telemetry.tracing.collect.sampling' = {}",
                tracing_sampling_rate
            );
            config =
                config.set_override("telemetry.tracing.collect.sampling", tracing_sampling_rate)?;
        }

        // Laboratory overrides
        if let Some(laboratory_enabled) = self.laboratory_enabled.take() {
            config = config.set_override("laboratory.enabled", laboratory_enabled)?;
        }

        if let Some(websocket_enabled) = self.websocket_enabled.take() {
            config = config.set_override("websocket.enabled", websocket_enabled)?;
        }

        if let Some(subscriptions_enabled) = self.subscriptions_enabled.take() {
            config = config.set_override("subscriptions.enabled", subscriptions_enabled)?;
        }

        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use config::{Config, File, FileFormat};

    use crate::HiveRouterConfig;

    use super::*;

    fn config_from_overrides(overrides: EnvVarOverrides) -> HiveRouterConfig {
        overrides
            .apply_overrides(Config::builder())
            .unwrap()
            .build()
            .unwrap()
            .try_deserialize::<HiveRouterConfig>()
            .unwrap()
    }

    #[test]
    fn tracing_sampling_rate_override_sets_tracing_collect_sampling() {
        let config = config_from_overrides(EnvVarOverrides {
            tracing_sampling_rate: Some(0.25),
            ..Default::default()
        });

        assert_eq!(config.telemetry.tracing.collect.sampling, 0.25);
    }

    #[test]
    fn tracing_sampling_rate_override_wins_over_config_file_value() {
        let config = EnvVarOverrides {
            tracing_sampling_rate: Some(0.1),
            ..Default::default()
        }
        .apply_overrides(Config::builder().add_source(File::from_str(
            r#"
telemetry:
  tracing:
    collect:
      sampling: 0.75
"#,
            FileFormat::Yaml,
        )))
        .unwrap()
        .build()
        .unwrap()
        .try_deserialize::<HiveRouterConfig>()
        .unwrap();

        assert_eq!(config.telemetry.tracing.collect.sampling, 0.1);
    }
}
