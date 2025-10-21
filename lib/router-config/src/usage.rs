use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct UsageConfig {
    /// Your [Registry Access Token](https://the-guild.dev/graphql/hive/docs/management/targets#registry-access-tokens) with write permission.
    pub token: String,
    /// A target ID, this can either be a slug following the format “$organizationSlug/$projectSlug/$targetSlug” (e.g “the-guild/graphql-hive/staging”) or an UUID (e.g. “a0f4c605-6541-4350-8cfe-b31f21a4bf80”). To be used when the token is configured with an organization access token.
    pub target_id: String,
    /// For self-hosting, you can override `/usage` endpoint (defaults to `https://app.graphql-hive.com/usage`).
    #[serde(default = "default_endpoint")]
    pub endpoint: String,
    /// Sample rate to determine sampling.
    /// 0.0 = 0% chance of being sent
    /// 1.0 = 100% chance of being sent
    /// Default: 1.0
    #[serde(default = "default_sample_rate")]
    pub sample_rate: f64,
    /// A list of operations (by name) to be ignored by Hive.
    /// Example: ["IntrospectionQuery", "MeQuery"]
    #[serde(default)]
    pub exclude: Vec<String>,
    #[serde(default = "default_client_name_header")]
    pub client_name_header: String,
    #[serde(default = "default_client_version_header")]
    pub client_version_header: String,
    /// A maximum number of operations to hold in a buffer before sending to Hive Console
    /// Default: 1000
    #[serde(default = "default_buffer_size")]
    pub buffer_size: usize,
    /// Accepts invalid SSL certificates
    /// Default: false
    #[serde(default = "default_accept_invalid_certs")]
    pub accept_invalid_certs: bool,
    /// A timeout for only the connect phase of a request to Hive Console
    /// Unit: seconds
    /// Default: 5 (s)
    #[serde(default = "default_connect_timeout")]
    pub connect_timeout: u64,
    /// A timeout for the entire request to Hive Console
    /// Unit: seconds
    /// Default: 15 (s)
    #[serde(default = "default_request_timeout")]
    pub request_timeout: u64,
    /// Frequency of flushing the buffer to the server
    /// Default: 5 seconds
    #[serde(default = "default_flush_interval")]
    pub flush_interval: u64,
}

fn default_endpoint() -> String {
    "https://app.graphql-hive.com/usage".to_string()
}

fn default_sample_rate() -> f64 {
    1.0
}

fn default_client_name_header() -> String {
    "graphql-client-name".to_string()
}

fn default_client_version_header() -> String {
    "graphql-client-version".to_string()
}

fn default_buffer_size() -> usize {
    1000
}

fn default_accept_invalid_certs() -> bool {
    false
}

fn default_request_timeout() -> u64 {
    15
}

fn default_connect_timeout() -> u64 {
    5
}

fn default_flush_interval() -> u64 {
    5
}
