pub mod authorization;
pub mod cors;
pub mod csrf;
mod env_overrides;
pub mod graphiql;
pub mod headers;
pub mod http_server;
pub mod jwt_auth;
pub mod log;
pub mod override_labels;
pub mod override_subgraph_urls;
pub mod primitives;
pub mod query_planner;
pub mod supergraph;
pub mod traffic_shaping;

use config::{Config, File, FileFormat, FileSourceFile};
use envconfig::Envconfig;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

use crate::{
    env_overrides::{EnvVarOverrides, EnvVarOverridesError},
    graphiql::GraphiQLConfig,
    http_server::HttpServerConfig,
    log::LoggingConfig,
    override_labels::OverrideLabelsConfig,
    primitives::file_path::with_start_path,
    query_planner::QueryPlannerConfig,
    supergraph::SupergraphSource,
    traffic_shaping::TrafficShapingConfig,
};

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HiveRouterConfig {
    #[serde(skip)]
    root_directory: PathBuf,

    /// The router logger configuration.
    ///
    /// The router is configured to be mostly silent (`info`) level, and will print only important messages, warnings, and errors.
    #[serde(default)]
    pub log: LoggingConfig,

    /// Configuration for the GraphiQL interface.
    #[serde(default)]
    pub graphiql: GraphiQLConfig,

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
}

#[derive(Debug, thiserror::Error)]
pub enum RouterConfigError {
    #[error("Failed to load configuration: {0}")]
    ConfigLoadError(#[from] config::ConfigError),
    #[error("Failed to apply configuration overrides: {0}")]
    EnvVarOverridesError(#[from] EnvVarOverridesError),
}

static DEFAULT_FILE_NAMES: &[&str] = &[
    "router.config.yaml",
    "router.config.yml",
    "router.config.json",
    "router.config.json5",
];

pub fn load_config(
    overide_config_path: Option<String>,
) -> Result<HiveRouterConfig, RouterConfigError> {
    let env_overrides = EnvVarOverrides::init_from_env().expect("failed to init env overrides");
    let mut config = Config::builder();
    let mut config_root_path = std::env::current_dir().expect("failed to get current directory");

    if let Some(path_str) = overide_config_path {
        let path_buf = path_str
            .parse::<std::path::PathBuf>()
            .expect("failed to parse config file path");
        let path_dupe = path_buf.clone();
        let parent_dir = path_dupe.parent().unwrap();
        let as_file: File<FileSourceFile, _> = path_buf.into();

        config = config.add_source(as_file.required(true));
        config_root_path = config_root_path.clone().join(parent_dir);
    } else {
        for name in DEFAULT_FILE_NAMES {
            config = config.add_source(File::with_name(name).required(false));
        }
    }

    config = env_overrides.apply_overrides(config)?;

    let mut base_cfg = with_start_path(&config_root_path, || {
        config.build()?.try_deserialize::<HiveRouterConfig>()
    })?;

    base_cfg.root_directory = config_root_path;

    Ok(base_cfg)
}

pub fn parse_yaml_config(config_raw: String) -> Result<HiveRouterConfig, config::ConfigError> {
    let config_root_path = std::env::current_dir().expect("failed to get current directory");
    let config = Config::builder();

    with_start_path(&config_root_path, || {
        config
            .add_source(File::from_str(&config_raw, FileFormat::Yaml))
            .build()?
            .try_deserialize::<HiveRouterConfig>()
    })
}
