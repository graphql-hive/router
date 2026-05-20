use std::collections::HashMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::primitives::http_header::HttpHeaderName;
use crate::primitives::ip_network::IpNetwork;
use crate::primitives::value_or_expression::ValueOrExpression;
use crate::telemetry::{hive::HiveTelemetryConfig, metrics::MetricsConfig, tracing::TracingConfig};

pub mod hive;
pub mod metrics;
pub mod tracing;

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
#[derive(Default)]
pub struct TelemetryConfig {
    #[serde(default)]
    pub hive: Option<HiveTelemetryConfig>,
    #[serde(default)]
    pub tracing: TracingConfig,
    #[serde(default)]
    pub metrics: MetricsConfig,
    #[serde(default)]
    pub resource: ResourceConfig,
    #[serde(default)]
    pub client_identification: ClientIdentificationConfig,
}

impl TelemetryConfig {
    pub fn is_tracing_enabled(&self) -> bool {
        self.tracing.is_enabled() || self.hive.as_ref().is_some_and(|hive| hive.tracing.enabled)
    }

    pub fn is_metrics_enabled(&self) -> bool {
        self.metrics.is_enabled()
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct ResourceConfig {
    #[serde(default)]
    pub attributes: HashMap<String, ValueOrExpression<String>>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct ClientIdentificationConfig {
    #[serde(default = "default_client_name_header")]
    pub name_header: HttpHeaderName,
    #[serde(default = "default_client_version_header")]
    pub version_header: HttpHeaderName,
    /// Defines how the client IP address is determined.
    ///
    /// Important: HTTP headers like `x-forwarded-for` can be spoofed by clients.
    /// Use it only with trusted proxies.
    ///
    /// It's null by default and uses the socket peer address.
    ///
    /// Use the left-most value from the specified header:
    /// ```ignore
    /// ip_header: "x-forwarded-for"
    /// ```
    ///
    /// If peer socket address is trusted, meaning it's part of `trusted_proxies` list,
    /// Router evaluates values from right to left and picks the first non-trusted value.
    /// If all values are trusted, uses the left-most value.
    /// ```ignore
    /// ip_header:
    ///   name: "x-forwarded-for"
    ///   trusted_proxies:
    ///     - 10.0.0.0/8
    ///     - 127.0.0.1/32
    /// ```
    #[serde(default)]
    pub ip_header: Option<ClientIpHeaderConfig>,
}

impl Default for ClientIdentificationConfig {
    fn default() -> Self {
        Self {
            name_header: default_client_name_header(),
            version_header: default_client_version_header(),
            ip_header: None,
        }
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(untagged)]
pub enum ClientIpHeaderConfig {
    HeaderName(HttpHeaderName),
    TrustedProxies(ClientIpHeaderTrustedProxiesConfig),
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct ClientIpHeaderTrustedProxiesConfig {
    /// Header name containing client and proxy chain values.
    pub name: HttpHeaderName,
    /// Trusted proxy addresses.
    /// Each entry can be an IP or CIDR.
    #[serde(default)]
    pub trusted_proxies: Vec<IpNetwork>,
}

fn default_client_name_header() -> HttpHeaderName {
    "graphql-client-name".into()
}

fn default_client_version_header() -> HttpHeaderName {
    "graphql-client-version".into()
}

#[cfg(test)]
mod tests {
    use super::{ClientIdentificationConfig, ClientIpHeaderConfig};

    #[test]
    fn client_identification_defaults_to_peer_address_source() {
        let config = ClientIdentificationConfig::default();
        assert!(config.ip_header.is_none());
    }

    #[test]
    fn client_identification_accepts_header_name_string() {
        let config = serde_json::from_str::<ClientIdentificationConfig>(
            r#"{"ip_header":"x-forwarded-for"}"#,
        )
        .expect("config should parse");

        match config.ip_header {
            Some(ClientIpHeaderConfig::HeaderName(name)) => {
                assert_eq!(name.get_header_ref().as_str(), "x-forwarded-for");
            }
            _ => panic!("expected header-name mode"),
        }
    }

    #[test]
    fn client_identification_accepts_trusted_proxy_object() {
        let config = serde_json::from_str::<ClientIdentificationConfig>(
            r#"{"ip_header":{"name":"x-forwarded-for","trusted_proxies":[]}}"#,
        )
        .expect("config should parse");

        match config.ip_header {
            Some(ClientIpHeaderConfig::TrustedProxies(config)) => {
                assert_eq!(config.name.get_header_ref().as_str(), "x-forwarded-for");
                assert_eq!(config.trusted_proxies, vec![]);
            }
            _ => panic!("expected trusted-proxies mode"),
        }
    }

    #[test]
    fn client_identification_rejects_non_ip_trusted_proxy_value() {
        let result = serde_json::from_str::<ClientIdentificationConfig>(
            r#"{"ip_header":{"name":"x-forwarded-for","trusted_proxies":["local"]}}"#,
        );
        assert!(result.is_err());
    }
}
