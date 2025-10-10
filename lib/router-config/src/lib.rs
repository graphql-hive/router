pub mod csrf;
pub mod headers;
pub mod http_server;
pub mod log;
pub mod primitives;
pub mod query_planner;
pub mod supergraph;
pub mod traffic_shaping;

use config::{Config, Environment, File, FileFormat, FileSourceFile};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    http_server::HttpServerConfig, log::LoggingConfig, query_planner::QueryPlannerConfig,
    supergraph::SupergraphSource, traffic_shaping::TrafficShapingExecutorConfig,
};

#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HiveRouterConfig {
    /// The router logger configuration.
    ///
    /// The router is configured to be mostly silent (`info`) level, and will print only important messages, warnings, and errors.
    #[serde(default)]
    pub log: LoggingConfig,

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

    /// Configuration for the traffic-shaper executor. Use these configurations to control how requests are being executed to subgraphs.
    #[serde(default)]
    pub traffic_shaping: TrafficShapingExecutorConfig,

    /// Configuration for the headers.
    #[serde(default)]
    pub headers: headers::HeadersConfig,

    /// Configuration for CSRF prevention.
    #[serde(default)]
    pub csrf: csrf::CSRFPreventionConfig,
}

#[derive(Debug, thiserror::Error)]
pub enum RouterConfigError {
    #[error("Failed to load configuration: {0}")]
    ConfigLoadError(config::ConfigError),
}

pub fn load_config(
    overide_config_path: Option<String>,
) -> Result<HiveRouterConfig, config::ConfigError> {
    let mut config = Config::builder();

    if let Some(path_str) = overide_config_path {
        let path_buf = path_str
            .parse::<std::path::PathBuf>()
            .expect("failed to parse config file path");
        let as_file: File<FileSourceFile, _> = path_buf.into();

        config = config.add_source(as_file.required(true));
    } else {
        config = config
            .add_source(File::with_name("hive-router.config.yaml").required(false))
            .add_source(File::with_name("hive-router.config.yml").required(false))
            .add_source(File::with_name("hive-router.config.json").required(false))
            .add_source(File::with_name("hive-router.config.json5").required(false))
    }

    config
        .add_source(
            Environment::with_prefix("HIVE")
                .separator("__")
                .prefix_separator("__"),
        )
        .build()?
        .try_deserialize::<HiveRouterConfig>()
}

pub fn parse_yaml_config(config_raw: String) -> Result<HiveRouterConfig, config::ConfigError> {
    Config::builder()
        .add_source(File::from_str(&config_raw, FileFormat::Yaml))
        .build()?
        .try_deserialize::<HiveRouterConfig>()
}
