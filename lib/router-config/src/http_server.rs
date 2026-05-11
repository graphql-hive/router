use std::num::NonZeroUsize;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct HttpServerConfig {
    /// The endpoint to serve GraphQL requests. By default, `/graphql` is used.
    #[serde(default = "graphql_endpoint_default")]
    pub graphql_endpoint: String,

    /// The host address to bind the HTTP server to.
    ///
    /// Can also be set via the `HOST` environment variable.
    #[serde(default = "http_server_host_default")]
    pub host: String,

    /// The port to bind the HTTP server to.
    ///
    /// Can also be set via the `PORT` environment variable.
    ///
    /// If you are running the router inside a Docker container, please ensure that the port is exposed correctly using `-p <host_port>:<container_port>` flag.
    #[serde(default = "http_server_port_default")]
    pub port: u16,

    /// The number of worker threads to use for the HTTP server. Must be at least `1`.
    ///
    /// Defaults to the number of physical CPU cores available to the process.
    ///
    /// Useful in containerized environments (e.g., Kubernetes) where the number of
    /// physical cores reported by the OS is higher than the actual CPU limit
    /// assigned to the container. In such cases, you should set this to match the
    /// container's CPU limit to avoid oversubscribing worker threads.
    ///
    /// Can also be set via the `ROUTER_HTTP_WORKERS` environment variable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workers: Option<NonZeroUsize>,
}

impl Default for HttpServerConfig {
    fn default() -> Self {
        Self {
            host: http_server_host_default(),
            port: http_server_port_default(),
            graphql_endpoint: graphql_endpoint_default(),
            workers: None,
        }
    }
}

fn http_server_host_default() -> String {
    "0.0.0.0".to_string()
}

fn graphql_endpoint_default() -> String {
    "/graphql".to_string()
}

fn http_server_port_default() -> u16 {
    4000
}
