pub mod authorization;
pub mod coprocessor;
pub mod cors;
pub mod csrf;
pub mod demand_control;
mod env_overrides;
pub mod error_masking;
mod from_env;
pub mod headers;
pub mod http_server;
pub mod introspection_policy;
pub mod jwt_auth;
pub mod laboratory;
pub mod limits;
pub mod log;
pub mod override_labels;
pub mod override_subgraph_urls;
pub mod persisted_documents;
pub mod primitives;
pub mod query_planner;
pub mod response_extensions;
pub mod schema_from_env;
pub mod storage;
pub mod subscriptions;
pub mod supergraph;
pub mod telemetry;
pub mod traffic_shaping;
pub mod usage_reporting;
pub mod websocket;

use config::{Config, File, FileFormat, FileSourceFile};
use envconfig::Envconfig;
pub use humantime_serde;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::{collections::HashMap, convert::Infallible};

use crate::error_masking::ErrorMaskingConfig;
use crate::storage::StorageConfigMap;
use crate::{
    env_overrides::{EnvVarOverrides, EnvVarOverridesError},
    http_server::HttpServerConfig,
    introspection_policy::IntrospectionPermissionConfig,
    laboratory::LaboratoryConfig,
    log::LoggingConfig,
    override_labels::OverrideLabelsConfig,
    primitives::file_path::with_start_path,
    query_planner::QueryPlannerConfig,
    supergraph::SupergraphSource,
    traffic_shaping::TrafficShapingConfig,
};

#[derive(Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HiveRouterConfig {
    #[serde(skip)]
    root_directory: PathBuf,

    /// Warnings collected while resolving `from_env` placeholders (e.g. a referenced
    /// environment variable that was not set). Populated after deserialization, since it's
    /// not part of the config file itself; not logged directly because config loading happens
    /// before the tracing subscriber (whose format/level comes from this very config) exists.
    #[serde(skip)]
    from_env_warnings: Vec<String>,

    /// The router logger configuration.
    ///
    /// The router is configured to be mostly silent (`info`) level, and will print only important messages, warnings, and errors.
    #[serde(default)]
    pub log: LoggingConfig,

    /// Configuration for the Hive Laboratory interface.
    #[serde(default)]
    pub laboratory: LaboratoryConfig,

    /// Configuration for the Federation supergraph source. By default, the router will use a local file-based supergraph source (`./supergraph.graphql`).
    /// Each source has a different set of configuration, depending on the source type.
    #[serde(default)]
    #[schemars(extend("type" = "object"))]
    pub supergraph: SupergraphSource,

    /// Query planning configuration.
    #[serde(default)]
    pub query_planner: QueryPlannerConfig,

    /// Configuration for the HTTP server/listener.
    #[serde(default)]
    pub http: HttpServerConfig,

    /// Configuration for the traffic-shaping of the executor. Use these configurations to control how requests are being executed to subgraphs.
    #[serde(default)]
    pub traffic_shaping: TrafficShapingConfig,

    /// Configuration for the headers.
    #[serde(default)]
    pub headers: headers::HeadersConfig,

    /// Configuration for propagating subgraph response's `extensions` to the client.
    #[serde(default)]
    pub response_extensions: response_extensions::ResponseExtensionsConfig,

    /// Configuration for CSRF prevention.
    #[serde(default)]
    pub csrf: csrf::CSRFPreventionConfig,

    /// Configuration for CORS (Cross-Origin Resource Sharing).
    #[serde(default)]
    pub cors: cors::CORSConfig,

    /// Configuration for JWT authentication plugin.
    #[serde(
        default = "jwt_auth::JwtAuthConfig::default",
        skip_serializing_if = "jwt_auth::JwtAuthConfig::is_jwt_auth_disabled"
    )]
    pub jwt: jwt_auth::JwtAuthConfig,

    /// Configuration for overriding subgraph URLs.
    #[serde(default)]
    pub override_subgraph_urls: override_subgraph_urls::OverrideSubgraphUrlsConfig,

    /// Configuration for overriding labels.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub override_labels: OverrideLabelsConfig,

    #[serde(default)]
    pub authorization: authorization::AuthorizationConfig,

    #[serde(default)]
    /// Configuration for checking the limits such as query depth, complexity, etc.
    pub limits: limits::LimitsConfig,

    /// Configuration to enable or disable introspection queries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub introspection: Option<IntrospectionPermissionConfig>,

    #[serde(default)]
    pub telemetry: telemetry::TelemetryConfig,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub demand_control: Option<demand_control::DemandControlConfig>,

    /// Configuration for custom plugins
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub plugins: HashMap<String, PluginConfig>,

    /// Configuration for subscriptions.
    #[serde(default)]
    pub subscriptions: subscriptions::SubscriptionsConfig,

    /// Configuration of router's WebSocket server.
    #[serde(default)]
    pub websocket: websocket::WebSocketConfig,

    /// Configuration for persisted documents extraction and resolution.
    #[serde(default)]
    pub persisted_documents: persisted_documents::PersistedDocumentsConfig,

    /// Configuration for coprocessor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coprocessor: Option<coprocessor::CoprocessorConfig>,

    /// Configuration for storage sources.
    ///
    /// Each key is a unique identifier for the storage source, that can later be references in other parts of the config file.
    ///
    /// Example:
    /// ```yaml
    /// storages:
    ///   my-s3:
    ///     type: s3
    ///     bucket: my-bucket
    ///     region: eu-west-1
    /// ```
    #[serde(default, skip_serializing_if = "StorageConfigMap::is_empty")]
    pub storages: StorageConfigMap,

    /// Configuration for error masking.
    #[serde(default)]
    pub error_masking: ErrorMaskingConfig,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginConfig {
    #[serde(default = "default_plugin_enabled")]
    pub enabled: bool,
    #[serde(default = "default_plugin_warn_on_error")]
    pub warn_on_error: bool,
    #[serde(default = "default_plugin_user_config")]
    pub config: serde_json::Value,
}

impl Default for PluginConfig {
    fn default() -> Self {
        Self {
            enabled: default_plugin_enabled(),
            warn_on_error: default_plugin_warn_on_error(),
            config: default_plugin_user_config(),
        }
    }
}

pub fn default_plugin_user_config() -> serde_json::Value {
    serde_json::Value::Object(serde_json::Map::new())
}

pub fn default_plugin_enabled() -> bool {
    true
}

pub fn default_plugin_warn_on_error() -> bool {
    false
}

impl HiveRouterConfig {
    pub fn into_static(self) -> &'static HiveRouterConfig {
        Box::leak(Box::new(self))
    }

    /// Warnings collected while resolving `from_env` placeholders.
    /// Since config is loaded before tracing is set up, some warnings may be logged before the
    /// tracing subscriber is set up.
    /// We collect it here so it will be available after tracing is set up.
    pub fn from_env_warnings(&self) -> &[String] {
        &self.from_env_warnings
    }

    pub fn address(&self) -> String {
        format!("{}:{}", self.http.host, self.http.port)
    }

    pub fn host(&self) -> String {
        self.http.host.clone()
    }

    pub fn port(&self) -> u16 {
        self.http.port
    }

    pub fn workers(&self) -> Option<std::num::NonZeroUsize> {
        self.http.workers
    }

    pub fn graphql_path(&self) -> &str {
        &self.http.graphql_endpoint
    }

    pub fn websocket_path(&self) -> Option<&str> {
        self.websocket.enabled.then(|| {
            self.websocket
                .path
                .as_ref()
                .map(|p| p.as_str())
                .unwrap_or_else(|| self.graphql_path())
        })
    }

    pub fn callback_conf(&self) -> Option<&subscriptions::CallbackConfig> {
        self.subscriptions.callback.as_ref()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RouterConfigError {
    #[error("Failed to load configuration: {0}")]
    ConfigLoadError(#[from] config::ConfigError),
    #[error("Failed to apply configuration overrides: {0}")]
    EnvVarOverridesError(#[from] EnvVarOverridesError),
    #[error("Failed to load the environment variables: {0}")]
    EnvVarLoadError(#[from] envconfig::Error),
    #[error("Failed to get the current directory: {0}")]
    CurrentDirError(std::io::Error),
    #[error("Failed to parse the configuration file path: {0}")]
    ConfigPathParseError(Infallible),
}

static DEFAULT_FILE_NAMES: &[&str] = &[
    "router.config.yaml",
    "router.config.yml",
    "router.config.json",
    "router.config.json5",
];

fn get_current_dir() -> Result<PathBuf, RouterConfigError> {
    std::env::current_dir().map_err(RouterConfigError::CurrentDirError)
}

pub fn load_config(
    overide_config_path: Option<String>,
) -> Result<HiveRouterConfig, RouterConfigError> {
    let env_overrides = EnvVarOverrides::init_from_env()?;
    let mut config = Config::builder();
    let mut config_root_path = get_current_dir()?;

    if let Some(path_str) = overide_config_path {
        let path_buf = path_str
            .parse::<std::path::PathBuf>()
            .map_err(RouterConfigError::ConfigPathParseError)?;
        let path_dupe = path_buf.clone();
        let parent_dir = path_dupe.parent().unwrap();
        let as_file: File<FileSourceFile, _> = path_buf.into();

        config = config.add_source(as_file.required(true));
        config_root_path = config_root_path.join(parent_dir);
    } else {
        for name in DEFAULT_FILE_NAMES {
            config = config.add_source(File::with_name(name).required(false));
        }
    }

    config = env_overrides.apply_overrides(config)?;

    let (mut base_cfg, from_env_warnings) = with_start_path(&config_root_path, || {
        let mut built = config.build()?;
        let from_env_warnings = from_env::resolve_from_env_placeholders(&mut built.cache);
        let cfg = built.try_deserialize::<HiveRouterConfig>()?;
        Ok::<_, config::ConfigError>((cfg, from_env_warnings))
    })?;

    base_cfg.root_directory = config_root_path;
    base_cfg.from_env_warnings = from_env_warnings;

    Ok(base_cfg)
}

pub fn parse_yaml_config(config_raw: String) -> Result<HiveRouterConfig, RouterConfigError> {
    let env_overrides = EnvVarOverrides::init_from_env()?;
    let config_root_path = get_current_dir()?;
    let mut config = Config::builder();
    config = env_overrides.apply_overrides(config)?;

    let (mut cfg, from_env_warnings) = with_start_path(&config_root_path, || {
        let mut built = config
            .add_source(File::from_str(&config_raw, FileFormat::Yaml))
            .build()?;
        let from_env_warnings = from_env::resolve_from_env_placeholders(&mut built.cache);
        let cfg = built.try_deserialize::<HiveRouterConfig>()?;
        Ok::<_, config::ConfigError>((cfg, from_env_warnings))
    })
    .map_err(RouterConfigError::ConfigLoadError)?;

    cfg.from_env_warnings = from_env_warnings;

    Ok(cfg)
}

#[cfg(test)]
mod plugin_config_from_env_tests {
    use super::*;

    // The plugin system hands plugins `plugins.<name>.config` as an opaque `serde_json::Value`
    // (see `PluginConfig`) - it never gets its own `Deserialize` impl at this layer, since each
    // plugin defines its own config shape independently. `from_env` resolution runs once over
    // the whole `config::Value` tree before that split happens, so it should apply just as well
    // inside a plugin's arbitrary config blob as it does for any statically-typed field.

    #[test]
    fn resolves_from_env_placeholders_nested_inside_plugin_config() {
        unsafe { std::env::set_var("FROM_ENV_PLUGIN_TEST", "resolved-value") };

        let yaml = r#"
plugins:
  my_plugin:
    config:
      nested:
        from_env: FROM_ENV_PLUGIN_TEST
      untouched: literal
      list:
        - a
        - from_env: FROM_ENV_PLUGIN_TEST
"#;
        let config = parse_yaml_config(yaml.to_string()).unwrap();
        let plugin = config.plugins.get("my_plugin").unwrap();

        assert_eq!(plugin.config["nested"], serde_json::json!("resolved-value"));
        assert_eq!(plugin.config["untouched"], serde_json::json!("literal"));
        assert_eq!(
            plugin.config["list"],
            serde_json::json!(["a", "resolved-value"])
        );

        unsafe { std::env::remove_var("FROM_ENV_PLUGIN_TEST") };
    }

    #[test]
    fn missing_env_var_inside_plugin_config_just_omits_the_key() {
        unsafe { std::env::remove_var("FROM_ENV_PLUGIN_TEST_MISSING") };

        let yaml = r#"
plugins:
  my_plugin:
    config:
      nested:
        from_env: FROM_ENV_PLUGIN_TEST_MISSING
      untouched: literal
"#;
        let config = parse_yaml_config(yaml.to_string()).unwrap();
        let plugin = config.plugins.get("my_plugin").unwrap();

        // A plugin's config is a bare `serde_json::Value`, not a struct with `#[serde(default)]`
        // fields, so there's no "fall back to a default" here - the key is just absent, same as
        // if the user never wrote it in the YAML at all.
        assert!(plugin.config.get("nested").is_none());
        assert_eq!(plugin.config["untouched"], serde_json::json!("literal"));
        assert_eq!(config.from_env_warnings().len(), 1);
        assert!(config.from_env_warnings()[0].contains("FROM_ENV_PLUGIN_TEST_MISSING"));
    }
}
