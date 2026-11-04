use std::borrow::Cow;
use std::fmt;
use std::num::NonZeroUsize;
use std::time::Duration;

use schemars::{json_schema, JsonSchema, Schema, SchemaGenerator};
use serde::de::{self, Unexpected, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
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

    /// The number of worker threads to use for the HTTP server.
    ///
    /// - `cpuset` (default): one worker per logical CPU the process is allowed to run on
    ///   (its CPU affinity mask, which follows the cgroup cpuset on Linux), capped by the
    ///   container's CPU limit (the cgroup CPU quota, such as Kubernetes'
    ///   `resources.limits.cpu`), rounded down, with a minimum of `1`. For example, a
    ///   `1500m` CPU limit results in a single worker.
    /// - `container`: the same as `cpuset`, accepted so existing configurations keep working.
    /// - A positive integer: an explicit number of workers.
    ///
    /// Set an explicit number to run more workers than the CPU limit allows.
    ///
    /// ```yaml
    /// http:
    ///   workers: 4
    /// ```
    ///
    /// Can also be set via the `ROUTER_HTTP_WORKERS` environment variable.
    #[serde(default)]
    pub workers: HttpWorkersConfig,

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
            workers: HttpWorkersConfig::default(),
            shutdown_timeout: http_server_shutdown_timeout_default(),
        }
    }
}

/// How the number of HTTP server worker threads is determined.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum HttpWorkersConfig {
    /// One worker per logical CPU in the process' CPU affinity mask (the cgroup cpuset on Linux),
    /// capped by the cgroup CPU quota.
    #[default]
    Cpuset,
    /// The same as `Cpuset`, accepted so existing configurations keep working.
    Container,
    /// An explicit number of workers.
    Count(NonZeroUsize),
}

impl HttpWorkersConfig {
    const CPUSET: &'static str = "cpuset";
    const CONTAINER: &'static str = "container";
}

impl Serialize for HttpWorkersConfig {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            HttpWorkersConfig::Cpuset => serializer.serialize_str(Self::CPUSET),
            HttpWorkersConfig::Container => serializer.serialize_str(Self::CONTAINER),
            HttpWorkersConfig::Count(count) => serializer.serialize_u64(count.get() as u64),
        }
    }
}

impl<'de> Deserialize<'de> for HttpWorkersConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(HttpWorkersConfigVisitor)
    }
}

struct HttpWorkersConfigVisitor;

impl HttpWorkersConfigVisitor {
    fn count<E: de::Error>(value: u64) -> Result<HttpWorkersConfig, E> {
        usize::try_from(value)
            .ok()
            .and_then(NonZeroUsize::new)
            .map(HttpWorkersConfig::Count)
            .ok_or_else(|| E::invalid_value(Unexpected::Unsigned(value), &Self))
    }
}

impl<'de> Visitor<'de> for HttpWorkersConfigVisitor {
    type Value = HttpWorkersConfig;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        write!(
            formatter,
            "a positive integer, \"{}\" or \"{}\"",
            HttpWorkersConfig::CPUSET,
            HttpWorkersConfig::CONTAINER
        )
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> Result<Self::Value, E> {
        Self::count(value)
    }

    // the `config` crate hands integers from YAML files over as `i64`
    fn visit_i64<E: de::Error>(self, value: i64) -> Result<Self::Value, E> {
        u64::try_from(value)
            .map_err(|_| E::invalid_value(Unexpected::Signed(value), &self))
            .and_then(Self::count)
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
        match value {
            HttpWorkersConfig::CPUSET => Ok(HttpWorkersConfig::Cpuset),
            HttpWorkersConfig::CONTAINER => Ok(HttpWorkersConfig::Container),
            // numeric strings come from the `ROUTER_HTTP_WORKERS` environment variable, and were
            // already accepted in config files back when `workers` was a plain integer
            _ => match value.parse::<u64>() {
                Ok(count) => Self::count(count),
                Err(_) => Err(E::invalid_value(Unexpected::Str(value), &self)),
            },
        }
    }

    // `workers: null` was accepted back when `workers` was optional
    fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
        Ok(HttpWorkersConfig::default())
    }

    fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
        Ok(HttpWorkersConfig::default())
    }
}

impl JsonSchema for HttpWorkersConfig {
    fn schema_name() -> Cow<'static, str> {
        "HttpWorkersConfig".into()
    }

    fn json_schema(_gen: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "oneOf": [
                {
                    "description": "One worker per logical CPU the process is allowed to run on (its CPU affinity mask, which follows the cgroup cpuset on Linux), capped by the container's CPU limit (the cgroup CPU quota), rounded down, with a minimum of `1`.",
                    "const": HttpWorkersConfig::CPUSET
                },
                {
                    "description": "The same as `cpuset`, accepted so existing configurations keep working.",
                    "const": HttpWorkersConfig::CONTAINER
                },
                {
                    "description": "An explicit number of workers.",
                    "type": "integer",
                    "format": "uint",
                    "minimum": 1
                }
            ]
        })
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
    use std::num::NonZeroUsize;
    use std::time::Duration;

    use super::{HttpServerConfig, HttpWorkersConfig};

    fn workers_from_json(json: &str) -> Result<HttpWorkersConfig, serde_json::Error> {
        serde_json::from_str::<HttpServerConfig>(&format!(r#"{{ "workers": {json} }}"#))
            .map(|config| config.workers)
    }

    #[test]
    fn workers_defaults_to_cpuset() {
        let config: HttpServerConfig =
            serde_json::from_str("{}").expect("empty config should deserialize");

        assert_eq!(config.workers, HttpWorkersConfig::Cpuset);
    }

    #[test]
    fn workers_accepts_modes_and_counts() {
        assert_eq!(
            workers_from_json(r#""cpuset""#).unwrap(),
            HttpWorkersConfig::Cpuset
        );
        assert_eq!(
            workers_from_json(r#""container""#).unwrap(),
            HttpWorkersConfig::Container
        );
        assert_eq!(
            workers_from_json("4").unwrap(),
            HttpWorkersConfig::Count(NonZeroUsize::new(4).unwrap())
        );
        assert_eq!(
            workers_from_json(r#""4""#).unwrap(),
            HttpWorkersConfig::Count(NonZeroUsize::new(4).unwrap())
        );
    }

    #[test]
    fn workers_null_falls_back_to_cpuset() {
        assert_eq!(
            workers_from_json("null").unwrap(),
            HttpWorkersConfig::Cpuset
        );
    }

    #[test]
    fn workers_rejects_zero_negative_and_unknown_values() {
        for json in ["0", r#""0""#, "-1", r#""host""#] {
            let err = workers_from_json(json).expect_err(json).to_string();
            assert!(
                err.contains(r#"expected a positive integer, "cpuset" or "container""#),
                "unexpected error for {json}: {err}"
            );
        }
    }

    #[test]
    fn workers_serializes_back_to_the_configured_value() {
        for (workers, expected) in [
            (HttpWorkersConfig::Cpuset, serde_json::json!("cpuset")),
            (HttpWorkersConfig::Container, serde_json::json!("container")),
            (
                HttpWorkersConfig::Count(NonZeroUsize::new(4).unwrap()),
                serde_json::json!(4),
            ),
        ] {
            assert_eq!(serde_json::to_value(workers).unwrap(), expected);
        }
    }

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
