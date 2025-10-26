use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HttpServerConfig {
    /// The host address to bind the HTTP server to.
    ///
    /// Can also be set via the `HOST` environment variable.
    #[serde(default = "http_server_host_default")]
    host: String,

    /// The port to bind the HTTP server to.
    ///
    /// Can also be set via the `PORT` environment variable.
    ///
    /// If you are running the router inside a Docker container, please ensure that the port is exposed correctly using `-p <host_port>:<container_port>` flag.
    #[serde(default = "http_server_port_default")]
    port: u16,
}

impl Default for HttpServerConfig {
    fn default() -> Self {
        Self {
            host: http_server_host_default(),
            port: http_server_port_default(),
        }
    }
}

fn http_server_host_default() -> String {
    "0.0.0.0".to_string()
}

fn http_server_port_default() -> u16 {
    4000
}

impl HttpServerConfig {
    pub fn address(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}
