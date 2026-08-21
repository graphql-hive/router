use std::num::NonZeroUsize;
use std::time::Duration;

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

    /// How long the HTTP server waits for a follow-up request on an idle
    /// keep-alive connection before closing it.
    ///
    /// Defaults to 5 seconds, which matches ntex's own default.
    ///
    /// When the router sits behind a reverse proxy, set this above that proxy's
    /// idle timeout so the proxy closes first. Closing first from the router
    /// leaves the proxy holding a socket the server has already dropped, which
    /// then fails the next reused request.
    ///
    /// ```yaml
    /// http:
    ///   keep_alive: 80s
    /// ```
    ///
    /// Can also be set via the `ROUTER_HTTP_KEEP_ALIVE` environment variable.
    #[serde(
        default = "http_server_keep_alive_default",
        deserialize_with = "humantime_serde::deserialize",
        serialize_with = "humantime_serde::serialize"
    )]
    #[schemars(with = "String")]
    pub keep_alive: Duration,
}

impl Default for HttpServerConfig {
    fn default() -> Self {
        Self {
            host: http_server_host_default(),
            port: http_server_port_default(),
            graphql_endpoint: graphql_endpoint_default(),
            workers: None,
            keep_alive: http_server_keep_alive_default(),
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

/// Matches ntex's own default HTTP/1 keep-alive, so configurations that omit
/// `keep_alive` keep their existing behaviour.
fn http_server_keep_alive_default() -> Duration {
    Duration::from_secs(5)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::HttpServerConfig;

    #[test]
    fn keep_alive_defaults_to_five_seconds() {
        let config: HttpServerConfig =
            serde_json::from_str("{}").expect("empty config should deserialize");

        assert_eq!(config.keep_alive, Duration::from_secs(5));
    }

    #[test]
    fn keep_alive_accepts_human_readable_durations() {
        let config: HttpServerConfig = serde_json::from_str(r#"{ "keep_alive": "80s" }"#)
            .expect("human readable duration should deserialize");

        assert_eq!(config.keep_alive, Duration::from_secs(80));
    }

    #[test]
    fn keep_alive_serializes_back_to_a_duration_string() {
        let config = HttpServerConfig {
            keep_alive: Duration::from_secs(80),
            ..Default::default()
        };

        let serialized = serde_json::to_value(&config)
            .expect("config should serialize")
            .get("keep_alive")
            .cloned();

        assert_eq!(serialized, Some(serde_json::json!("1m 20s")));
    }
}
