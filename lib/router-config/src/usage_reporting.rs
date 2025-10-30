use std::{fmt::Display, time::Duration};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct UsageReportingConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Your [Registry Access Token](https://the-guild.dev/graphql/hive/docs/management/targets#registry-access-tokens) with write permission.
    pub access_token: String,
    /// A target ID, this can either be a slug following the format “$organizationSlug/$projectSlug/$targetSlug” (e.g “the-guild/graphql-hive/staging”) or an UUID (e.g. “a0f4c605-6541-4350-8cfe-b31f21a4bf80”). To be used when the token is configured with an organization access token.
    #[serde(deserialize_with = "deserialize_target_id")]
    pub target_id: Option<String>,
    /// For self-hosting, you can override `/usage` endpoint (defaults to `https://app.graphql-hive.com/usage`).
    #[serde(default = "default_endpoint")]
    pub endpoint: String,
    /// Sample rate to determine sampling.
    /// 0% = never being sent
    /// 50% = half of the requests being sent
    /// 100% = always being sent
    /// Default: 100%
    #[serde(default = "default_sample_rate")]
    #[schemars(with = "String")]
    pub sample_rate: Percentage,
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
    /// Default: 5 seconds
    #[serde(
        default = "default_connect_timeout",
        deserialize_with = "humantime_serde::deserialize",
        serialize_with = "humantime_serde::serialize"
    )]
    #[schemars(with = "String")]
    pub connect_timeout: Duration,
    /// A timeout for the entire request to Hive Console
    /// Default: 15 seconds
    #[serde(
        default = "default_request_timeout",
        deserialize_with = "humantime_serde::deserialize",
        serialize_with = "humantime_serde::serialize"
    )]
    #[schemars(with = "String")]
    pub request_timeout: Duration,
    /// Frequency of flushing the buffer to the server
    /// Default: 5 seconds
    #[serde(
        default = "default_flush_interval",
        deserialize_with = "humantime_serde::deserialize",
        serialize_with = "humantime_serde::serialize"
    )]
    #[schemars(with = "String")]
    pub flush_interval: Duration,
}

impl Default for UsageReportingConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            access_token: String::new(),
            target_id: None,
            endpoint: default_endpoint(),
            sample_rate: default_sample_rate(),
            exclude: Vec::new(),
            client_name_header: default_client_name_header(),
            client_version_header: default_client_version_header(),
            buffer_size: default_buffer_size(),
            accept_invalid_certs: default_accept_invalid_certs(),
            connect_timeout: default_connect_timeout(),
            request_timeout: default_request_timeout(),
            flush_interval: default_flush_interval(),
        }
    }
}

fn default_enabled() -> bool {
    false
}

fn default_endpoint() -> String {
    "https://app.graphql-hive.com/usage".to_string()
}

fn default_sample_rate() -> Percentage {
    Percentage::from_f64(1.0).unwrap()
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

fn default_request_timeout() -> Duration {
    Duration::from_secs(15)
}

fn default_connect_timeout() -> Duration {
    Duration::from_secs(5)
}

fn default_flush_interval() -> Duration {
    Duration::from_secs(5)
}

// Target ID regexp for validation: slug format
const TARGET_ID_SLUG_REGEX: &str = r"^[a-zA-Z0-9-_]+\/[a-zA-Z0-9-_]+\/[a-zA-Z0-9-_]+$";
// Target ID regexp for validation: UUID format
const TARGET_ID_UUID_REGEX: &str =
    r"^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$";

fn deserialize_target_id<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let opt = Option::<String>::deserialize(deserializer)?;
    if let Some(ref s) = opt {
        let trimmed_s = s.trim();
        if trimmed_s.is_empty() {
            Ok(None)
        } else {
            let slug_regex =
                regex_automata::meta::Regex::new(TARGET_ID_SLUG_REGEX).map_err(|err| {
                    serde::de::Error::custom(format!(
                        "Failed to compile target_id slug regex: {}",
                        err
                    ))
                })?;
            if slug_regex.is_match(trimmed_s) {
                return Ok(Some(trimmed_s.to_string()));
            }
            let uuid_regex =
                regex_automata::meta::Regex::new(TARGET_ID_UUID_REGEX).map_err(|err| {
                    serde::de::Error::custom(format!(
                        "Failed to compile target_id UUID regex: {}",
                        err
                    ))
                })?;
            if uuid_regex.is_match(trimmed_s) {
                return Ok(Some(trimmed_s.to_string()));
            }
            Err(serde::de::Error::custom(format!(
                "Invalid target_id format: '{}'. It must be either in slug format '$organizationSlug/$projectSlug/$targetSlug' or UUID format 'a0f4c605-6541-4350-8cfe-b31f21a4bf80'",
                trimmed_s
            )))
        }
    } else {
        Ok(None)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Percentage {
    value: f64,
}

impl Percentage {
    pub fn from_str(s: &str) -> Result<Self, String> {
        let s_trimmed = s.trim();
        if let Some(number_part) = s_trimmed.strip_suffix('%') {
            let value: f64 = number_part.parse().map_err(|err| {
                format!(
                    "Failed to parse percentage value '{}': {}",
                    number_part, err
                )
            })?;
            Ok(Percentage::from_f64(value / 100.0)?)
        } else {
            Err(format!(
                "Percentage value must end with '%', got: '{}'",
                s_trimmed
            ))
        }
    }
    pub fn from_f64(value: f64) -> Result<Self, String> {
        if !(0.0..=1.0).contains(&value) {
            return Err(format!(
                "Percentage value must be between 0 and 1, got: {}",
                value
            ));
        }
        Ok(Percentage { value })
    }
    pub fn as_f64(&self) -> f64 {
        self.value
    }
}

impl Display for Percentage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}%", self.value * 100.0)
    }
}   

// Deserializer from `n%` string to `Percentage` struct
impl<'de> Deserialize<'de> for Percentage {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Percentage::from_str(&s).map_err(serde::de::Error::custom)
    }
}

// Serializer from `Percentage` struct to `n%` string
impl Serialize for Percentage {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}
