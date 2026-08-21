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

    /// How long the HTTP server waits for in-flight requests to complete after
    /// receiving a termination signal (`SIGTERM`), before remaining workers are
    /// force-dropped.
    ///
    /// Defaults to 30 seconds.
    ///
    /// Set this above `traffic_shaping.router.request_timeout` if you need the
    /// longest requests the router accepts to also survive a shutdown. In
    /// orchestrated environments the platform's own grace period (for example
    /// Kubernetes' `terminationGracePeriodSeconds`) must in turn exceed this
    /// value, otherwise the process is killed before the drain completes.
    ///
    /// ```yaml
    /// http:
    ///   shutdown_timeout: 90s
    /// ```
    ///
    /// Can also be set via the `ROUTER_HTTP_SHUTDOWN_TIMEOUT` environment variable.
    #[serde(
        default = "http_server_shutdown_timeout_default",
        deserialize_with = "humantime_serde::deserialize",
        serialize_with = "humantime_serde::serialize"
    )]
    #[schemars(with = "String")]
    pub shutdown_timeout: Duration,
}

impl Default for HttpServerConfig {
    fn default() -> Self {
        Self {
            host: http_server_host_default(),
            port: http_server_port_default(),
            graphql_endpoint: graphql_endpoint_default(),
            workers: None,
            shutdown_timeout: http_server_shutdown_timeout_default(),
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

/// Matches ntex's own default graceful shutdown timeout, so configurations that
/// omit `shutdown_timeout` keep their existing behaviour.
fn http_server_shutdown_timeout_default() -> Duration {
    Duration::from_secs(30)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::HttpServerConfig;

    #[test]
    fn shutdown_timeout_defaults_to_thirty_seconds() {
        let config: HttpServerConfig =
            serde_json::from_str("{}").expect("empty config should deserialize");

        assert_eq!(config.shutdown_timeout, Duration::from_secs(30));
    }

    #[test]
    fn shutdown_timeout_accepts_human_readable_durations() {
        let config: HttpServerConfig = serde_json::from_str(r#"{ "shutdown_timeout": "1m30s" }"#)
            .expect("human readable duration should deserialize");

        assert_eq!(config.shutdown_timeout, Duration::from_secs(90));
    }

    #[test]
    fn shutdown_timeout_serializes_back_to_a_duration_string() {
        let config = HttpServerConfig {
            shutdown_timeout: Duration::from_secs(90),
            ..Default::default()
        };

        let serialized = serde_json::to_value(&config)
            .expect("config should serialize")
            .get("shutdown_timeout")
            .cloned();

        assert_eq!(serialized, Some(serde_json::json!("1m 30s")));
    }
}
