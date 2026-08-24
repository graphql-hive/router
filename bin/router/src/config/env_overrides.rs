use config_rs::{builder::BuilderState, ConfigBuilder, ConfigError};
use envconfig::Envconfig;
use tracing::debug;

use crate::config::log::{LogFormat, LogLevel};

#[derive(Default, Envconfig)]
pub struct EnvVarOverrides {
    // Logger overrides
    #[envconfig(from = "LOG_LEVEL")]
    pub log_level: Option<LogLevel>,
    #[envconfig(from = "LOG_FORMAT")]
    pub log_format: Option<LogFormat>,
    #[envconfig(from = "LOG_FILTER")]
    pub log_filter: Option<String>,
    #[envconfig(from = "LOG_INTERNALS")]
    pub log_internals: Option<bool>,

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
    #[envconfig(from = "ROUTER_HTTP_KEEP_ALIVE")]
    pub http_keep_alive: Option<String>,
    #[envconfig(from = "ROUTER_HTTP_SHUTDOWN_TIMEOUT")]
    pub http_shutdown_timeout: Option<String>,

    // Supergraph overrides
    #[envconfig(from = "SUPERGRAPH_FILE_PATH")]
    pub supergraph_file_path: Option<String>,
    #[envconfig(from = "HIVE_CDN_ENDPOINT")]
    pub hive_console_cdn_endpoint: Option<String>,
    #[envconfig(from = "HIVE_CDN_KEY")]
    pub hive_console_cdn_key: Option<String>,
    #[envconfig(from = "HIVE_CDN_POLL_INTERVAL")]
    pub hive_console_cdn_poll_interval: Option<String>,
    #[envconfig(from = "APOLLO_KEY")]
    pub apollo_key: Option<String>,
    #[envconfig(from = "APOLLO_GRAPH_REF")]
    pub apollo_graph_ref: Option<String>,
    #[envconfig(from = "APOLLO_UPLINK_ENDPOINTS")]
    pub apollo_uplink_endpoints: Option<String>,
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

    // Query planner overrides
    #[envconfig(from = "QUERY_PLANNER_EXPERIMENTAL_ABSTRACT_TYPE_FOLDING")]
    pub query_planner_experimental_abstract_type_folding: Option<bool>,

    // Error masking
    #[envconfig(from = "DISABLE_SUBGRAPH_ERROR_MASKING")]
    pub disable_subgraph_error_masking: Option<bool>,
}

#[derive(Debug, thiserror::Error)]
pub enum EnvVarOverridesError {
    #[error("Failed to override configuration: {0}")]
    FailedToOverrideConfig(#[from] ConfigError),
    #[error("Cannot override supergraph source due to conflict: SUPERGRAPH_FILE_PATH, HIVE_CDN_ENDPOINT and APOLLO_KEY cannot be used together")]
    ConflictingSupergraphSource,
    #[error("Missing required environment variable: {0}")]
    MissingRequiredEnvVar(&'static str),
}

const CONFIG_LOGGING_TARGET: &str = "router::config";

impl EnvVarOverrides {
    pub fn apply_overrides<T: BuilderState>(
        mut self,
        mut config: ConfigBuilder<T>,
    ) -> Result<ConfigBuilder<T>, EnvVarOverridesError> {
        if let Some(log_level) = self.log_level.take() {
            debug!(target: CONFIG_LOGGING_TARGET, value = ?log_level, "overriding 'log.level'");
            config = config.set_override("log.level", log_level.as_str())?;
        }
        if let Some(log_format) = self.log_format.take() {
            debug!(target: CONFIG_LOGGING_TARGET, value = ?log_format, "overriding 'log.format'");
            config = config.set_override("log.format", log_format.as_str())?;
        }
        if let Some(log_filter) = self.log_filter.take() {
            debug!(target: CONFIG_LOGGING_TARGET, value = ?log_filter, "overriding 'log.filter'");
            config = config.set_override("log.filter", log_filter)?;
        }
        if let Some(log_internals) = self.log_internals.take() {
            debug!(target: CONFIG_LOGGING_TARGET, value = log_internals, "overriding 'log.log_internals'");
            config = config.set_override("log.log_internals", log_internals)?;
        }

        if let Some(http_port) = self.http_port.take() {
            debug!(target: CONFIG_LOGGING_TARGET, value = http_port, "overriding 'http.port'");
            config = config.set_override("http.port", http_port)?;
        }

        if let Some(http_host) = self.http_host.take() {
            debug!(target: CONFIG_LOGGING_TARGET, value = http_host, "overriding 'http.host'");
            config = config.set_override("http.host", http_host)?;
        }

        if let Some(http_workers) = self.http_workers.take() {
            debug!(target: CONFIG_LOGGING_TARGET, value = http_workers, "overriding 'http.workers'");
            // cast to u64 because the `config` crate doesn't implement `Into<Value>` for `usize`;
            // the value is then deserialized into `Option<NonZeroUsize>`, which rejects `0`.
            config = config.set_override("http.workers", http_workers as u64)?;
        }

        if let Some(http_keep_alive) = self.http_keep_alive.take() {
            debug!(target: CONFIG_LOGGING_TARGET, value = http_keep_alive, "overriding 'traffic_shaping.router.keep_alive'");
            config = config.set_override("traffic_shaping.router.keep_alive", http_keep_alive)?;
        }

        if let Some(http_shutdown_timeout) = self.http_shutdown_timeout.take() {
            debug!(target: CONFIG_LOGGING_TARGET, value = http_shutdown_timeout, "overriding 'http.shutdown_timeout'");
            config = config.set_override("http.shutdown_timeout", http_shutdown_timeout)?;
        }

        let configured_supergraph_sources = [
            self.supergraph_file_path.is_some(),
            self.hive_console_cdn_endpoint.is_some(),
            self.apollo_key.is_some(),
        ]
        .into_iter()
        .filter(|configured| *configured)
        .count();
        if configured_supergraph_sources > 1 {
            return Err(EnvVarOverridesError::ConflictingSupergraphSource);
        }

        if let Some(supergraph_file_path) = self.supergraph_file_path.take() {
            debug!(target: CONFIG_LOGGING_TARGET, value = supergraph_file_path, "overriding 'supergraph.path'");
            config = config.set_override("supergraph.source", "file")?;
            config = config.set_override("supergraph.path", supergraph_file_path)?;
        }

        if let Some(hive_console_cdn_endpoint) = self.hive_console_cdn_endpoint.take() {
            debug!(target: CONFIG_LOGGING_TARGET, value = hive_console_cdn_endpoint, "overriding 'hive_console_cdn_endpoint'");
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
                debug!(target: CONFIG_LOGGING_TARGET, "overriding 'hive_console_cdn_key'");
                config = config.set_override("supergraph.key", hive_console_cdn_key)?;
            } else {
                return Err(EnvVarOverridesError::MissingRequiredEnvVar("HIVE_CDN_KEY"));
            }

            if let Some(hive_console_cdn_poll_interval) = self.hive_console_cdn_poll_interval.take()
            {
                debug!(target: CONFIG_LOGGING_TARGET, value = hive_console_cdn_poll_interval, "overriding 'hive_console_cdn_poll_interval'");
                config = config
                    .set_override("supergraph.poll_interval", hive_console_cdn_poll_interval)?;
            }
        }

        if let Some(apollo_key) = self.apollo_key.take() {
            debug!(target: CONFIG_LOGGING_TARGET, "overriding 'apollo_key'");
            config = config.set_override("supergraph.source", "apollo_graphos")?;
            config = config.set_override("supergraph.key", apollo_key)?;

            if let Some(apollo_graph_ref) = self.apollo_graph_ref.take() {
                debug!(target: CONFIG_LOGGING_TARGET, value = apollo_graph_ref, "overriding 'apollo_graph_ref'");
                config = config.set_override("supergraph.graph_ref", apollo_graph_ref)?;
            } else {
                return Err(EnvVarOverridesError::MissingRequiredEnvVar(
                    "APOLLO_GRAPH_REF",
                ));
            }

            if let Some(apollo_uplink_endpoints) = self.apollo_uplink_endpoints.take() {
                debug!(target: CONFIG_LOGGING_TARGET, value = apollo_uplink_endpoints, "overriding 'apollo_uplink_endpoints'");
                let endpoints: Vec<String> = apollo_uplink_endpoints
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .collect();
                config = config.set_override("supergraph.endpoint", endpoints)?;
            }
        }

        if let Some(enabled) = self.hive_tracing_enabled.take() {
            debug!(target: CONFIG_LOGGING_TARGET, value = enabled, "overriding 'hive_tracing_enabled'");
            config = config.set_override("telemetry.hive.tracing.enabled", enabled)?;
        }

        if let Some(enabled) = self.hive_usage_reporting_enabled.take() {
            debug!(target: CONFIG_LOGGING_TARGET, value = enabled, "overriding 'hive_usage_reporting_enabled'");
            config = config.set_override("telemetry.hive.usage_reporting.enabled", enabled)?;
        }

        if let Some(hive_access_token) = self.hive_access_token.take() {
            debug!(target: CONFIG_LOGGING_TARGET, "overriding 'hive_access_token'");
            config = config.set_override("telemetry.hive.token", hive_access_token)?;
        }

        if let Some(hive_target) = self.hive_target.take() {
            debug!(target: CONFIG_LOGGING_TARGET, value = hive_target, "overriding 'hive_target'");
            config = config.set_override("telemetry.hive.target", hive_target)?;
        }

        if let Some(tracing_sampling_rate) = self.tracing_sampling_rate.take() {
            debug!(target: CONFIG_LOGGING_TARGET, value = tracing_sampling_rate, "overriding 'tracing_sampling_rate'");

            config =
                config.set_override("telemetry.tracing.collect.sampling", tracing_sampling_rate)?;
        }

        // Laboratory overrides
        if let Some(laboratory_enabled) = self.laboratory_enabled.take() {
            debug!(target: CONFIG_LOGGING_TARGET, value = laboratory_enabled, "overriding 'laboratory_enabled'");
            config = config.set_override("laboratory.enabled", laboratory_enabled)?;
        }

        if let Some(websocket_enabled) = self.websocket_enabled.take() {
            debug!(target: CONFIG_LOGGING_TARGET, value = websocket_enabled, "overriding 'websocket_enabled'");
            config = config.set_override("websocket.enabled", websocket_enabled)?;
        }

        if let Some(subscriptions_enabled) = self.subscriptions_enabled.take() {
            debug!(target: CONFIG_LOGGING_TARGET, value = subscriptions_enabled, "overriding 'subscriptions_enabled'");
            config = config.set_override("subscriptions.enabled", subscriptions_enabled)?;
        }

        if let Some(experimental_abstract_type_folding) =
            self.query_planner_experimental_abstract_type_folding.take()
        {
            debug!(target: CONFIG_LOGGING_TARGET, value = experimental_abstract_type_folding, "overriding 'experimental_abstract_type_folding'");
            config = config.set_override(
                "query_planner.experimental_abstract_type_folding",
                experimental_abstract_type_folding,
            )?;
        }

        if let Some(disable_subgraph_error_masking) = self.disable_subgraph_error_masking.take() {
            debug!(
                "[config-override] 'disable_subgraph_error_masking' = {}",
                disable_subgraph_error_masking
            );

            config =
                config.set_override("error_masking.enabled", !disable_subgraph_error_masking)?;
        }

        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use config_rs::{Config, File, FileFormat};

    use crate::config::HiveRouterConfig;

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

    #[test]
    fn http_keep_alive_override_sets_http_keep_alive() {
        let config = config_from_overrides(EnvVarOverrides {
            http_keep_alive: Some("80s".to_string()),
            ..Default::default()
        });

        assert_eq!(
            config.traffic_shaping.router.keep_alive,
            std::time::Duration::from_secs(80)
        );
    }

    #[test]
    fn http_keep_alive_override_wins_over_config_file_value() {
        let config = EnvVarOverrides {
            http_keep_alive: Some("80s".to_string()),
            ..Default::default()
        }
        .apply_overrides(Config::builder().add_source(File::from_str(
            r#"
traffic_shaping:
  router:
    keep_alive: 5s
"#,
            FileFormat::Yaml,
        )))
        .unwrap()
        .build()
        .unwrap()
        .try_deserialize::<HiveRouterConfig>()
        .unwrap();

        assert_eq!(
            config.traffic_shaping.router.keep_alive,
            std::time::Duration::from_secs(80)
        );
    }

    #[test]
    fn http_shutdown_timeout_override_sets_http_shutdown_timeout() {
        let config = config_from_overrides(EnvVarOverrides {
            http_shutdown_timeout: Some("90s".to_string()),
            ..Default::default()
        });

        assert_eq!(
            config.http.shutdown_timeout,
            std::time::Duration::from_secs(90)
        );
    }

    #[test]
    fn http_shutdown_timeout_override_wins_over_config_file_value() {
        let config = EnvVarOverrides {
            http_shutdown_timeout: Some("90s".to_string()),
            ..Default::default()
        }
        .apply_overrides(Config::builder().add_source(File::from_str(
            r#"
http:
  shutdown_timeout: 45s
"#,
            FileFormat::Yaml,
        )))
        .unwrap()
        .build()
        .unwrap()
        .try_deserialize::<HiveRouterConfig>()
        .unwrap();

        assert_eq!(
            config.http.shutdown_timeout,
            std::time::Duration::from_secs(90)
        );
    }

    #[test]
    fn query_planner_experimental_abstract_type_folding_override_sets_config() {
        let config = config_from_overrides(EnvVarOverrides {
            query_planner_experimental_abstract_type_folding: Some(true),
            ..Default::default()
        });

        assert!(config.query_planner.experimental_abstract_type_folding);
    }

    #[test]
    fn query_planner_experimental_abstract_type_folding_override_wins_over_config_file_value() {
        let config = EnvVarOverrides {
            query_planner_experimental_abstract_type_folding: Some(true),
            ..Default::default()
        }
        .apply_overrides(Config::builder().add_source(File::from_str(
            r#"
query_planner:
  experimental_abstract_type_folding: false
"#,
            FileFormat::Yaml,
        )))
        .unwrap()
        .build()
        .unwrap()
        .try_deserialize::<HiveRouterConfig>()
        .unwrap();

        assert!(config.query_planner.experimental_abstract_type_folding);
    }
}
