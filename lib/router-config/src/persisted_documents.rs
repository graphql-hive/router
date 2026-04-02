use schemars::JsonSchema;
use serde::{de::Error as _, Deserialize, Serialize};
use std::collections::HashSet;
use std::time::Duration;

use crate::primitives::file_path::FilePath;
use crate::primitives::retry_policy::RetryPolicyConfig;
use crate::primitives::single_or_multiple::SingleOrMultiple;
use crate::primitives::toggle::ToggleWith;

#[derive(Debug, Serialize, JsonSchema, Clone, Default)]
pub struct PersistedDocumentsConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub require_id: bool,
    #[serde(default)]
    pub log_missing_id: bool,
    #[serde(default)]
    pub storage: Option<PersistedDocumentsStorageConfig>,
    #[serde(default)]
    pub selectors: Option<Vec<PersistedDocumentExtractorConfig>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPersistedDocumentsConfig {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    require_id: bool,
    #[serde(default)]
    log_missing_id: bool,
    #[serde(default)]
    storage: Option<PersistedDocumentsStorageConfig>,
    #[serde(default)]
    selectors: Option<Vec<PersistedDocumentExtractorConfig>>,
}

impl<'de> Deserialize<'de> for PersistedDocumentsConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = RawPersistedDocumentsConfig::deserialize(deserializer)?;

        if raw.enabled && matches!(raw.selectors.as_ref(), Some(selectors) if selectors.is_empty())
        {
            return Err(D::Error::custom(
                "persisted_documents.selectors must not be an explicit empty list when persisted_documents.enabled=true",
            ));
        }

        if raw.enabled && raw.storage.is_none() {
            return Err(D::Error::custom(
                "persisted_documents.storage is required when persisted_documents.enabled=true",
            ));
        }

        if let Some(selectors) = raw.selectors.as_ref() {
            let mut seen = HashSet::new();
            for selector in selectors {
                if !seen.insert(selector.clone()) {
                    return Err(D::Error::custom(format!(
                        "persisted_documents.selectors contains a duplicate entry: {selector:?}"
                    )));
                }
            }
        }

        Ok(Self {
            enabled: raw.enabled,
            require_id: raw.require_id,
            log_missing_id: raw.log_missing_id,
            storage: raw.storage,
            selectors: raw.selectors,
        })
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields, tag = "type", rename_all = "snake_case")]
pub enum PersistedDocumentsStorageConfig {
    File {
        #[serde(flatten)]
        config: PersistedDocumentsFileStorageConfig,
    },
    Hive {
        #[serde(flatten)]
        config: PersistedDocumentsHiveStorageConfig,
    },
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct PersistedDocumentsFileStorageConfig {
    pub path: FilePath,
    #[serde(default = "default_watch")]
    pub watch: bool,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct PersistedDocumentsHiveStorageConfig {
    /// The CDN endpoint from Hive Console target.
    /// Can also be set using the `HIVE_CDN_ENDPOINT` environment variable.
    pub endpoint: Option<SingleOrMultiple<String>>,
    /// The CDN Access Token with from the Hive Console target.
    /// Can also be set using the `HIVE_CDN_KEY` environment variable.
    pub key: Option<String>,
    #[serde(default = "default_hive_accept_invalid_certs")]
    pub accept_invalid_certs: bool,
    #[serde(
        default = "default_hive_connect_timeout",
        deserialize_with = "humantime_serde::deserialize",
        serialize_with = "humantime_serde::serialize"
    )]
    #[schemars(with = "String")]
    pub connect_timeout: Duration,
    #[serde(
        default = "default_hive_request_timeout",
        deserialize_with = "humantime_serde::deserialize",
        serialize_with = "humantime_serde::serialize"
    )]
    #[schemars(with = "String")]
    pub request_timeout: Duration,
    #[serde(default = "default_hive_retry_policy")]
    pub retry_policy: RetryPolicyConfig,
    #[serde(default = "default_hive_cache_size")]
    pub cache_size: u64,
    #[serde(default)]
    pub circuit_breaker: PersistedDocumentsHiveCircuitBreakerConfig,
    #[serde(default = "default_hive_negative_cache")]
    pub negative_cache: ToggleWith<PersistedDocumentsHiveNegativeCacheConfig>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PersistedDocumentsHiveNegativeCacheConfig {
    #[serde(
        deserialize_with = "humantime_serde::deserialize",
        serialize_with = "humantime_serde::serialize"
    )]
    #[schemars(with = "String")]
    pub ttl: Duration,
}

impl Default for PersistedDocumentsHiveNegativeCacheConfig {
    fn default() -> Self {
        Self {
            ttl: Duration::from_secs(5),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct PersistedDocumentsHiveCircuitBreakerConfig {
    #[serde(default = "default_circuit_breaker_error_threshold")]
    pub error_threshold: f32,
    #[serde(default = "default_circuit_breaker_volume_threshold")]
    pub volume_threshold: usize,
    #[serde(
        default = "default_circuit_breaker_reset_timeout",
        deserialize_with = "humantime_serde::deserialize",
        serialize_with = "humantime_serde::serialize"
    )]
    #[schemars(with = "String")]
    pub reset_timeout: Duration,
}

impl Default for PersistedDocumentsHiveCircuitBreakerConfig {
    fn default() -> Self {
        Self {
            error_threshold: default_circuit_breaker_error_threshold(),
            volume_threshold: default_circuit_breaker_volume_threshold(),
            reset_timeout: default_circuit_breaker_reset_timeout(),
        }
    }
}

fn default_hive_accept_invalid_certs() -> bool {
    false
}

fn default_hive_connect_timeout() -> Duration {
    Duration::from_secs(5)
}

fn default_hive_request_timeout() -> Duration {
    Duration::from_secs(15)
}

fn default_hive_retry_policy() -> RetryPolicyConfig {
    RetryPolicyConfig { max_retries: 3 }
}

fn default_hive_cache_size() -> u64 {
    10_000
}

fn default_hive_negative_cache() -> ToggleWith<PersistedDocumentsHiveNegativeCacheConfig> {
    ToggleWith::Enabled(PersistedDocumentsHiveNegativeCacheConfig::default())
}

fn default_circuit_breaker_error_threshold() -> f32 {
    0.5
}

fn default_circuit_breaker_volume_threshold() -> usize {
    5
}

fn default_circuit_breaker_reset_timeout() -> Duration {
    Duration::from_secs(10)
}

const fn default_watch() -> bool {
    true
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, PartialEq, Eq, Hash)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PersistedDocumentExtractorConfig {
    JsonPath {
        path: PersistedDocumentJsonPath,
    },
    UrlPathParam {
        template: PersistedDocumentUrlTemplate,
    },
    UrlQueryParam {
        name: PersistedDocumentQueryParamName,
    },
}

#[derive(Debug, Serialize, JsonSchema, Clone, PartialEq, Eq, Hash)]
#[serde(transparent)]
pub struct PersistedDocumentJsonPath(String);

impl PersistedDocumentJsonPath {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for PersistedDocumentJsonPath {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        // TODO: add more validations (like " char etc)
        let path = String::deserialize(deserializer)?;
        if path.is_empty() {
            return Err(D::Error::custom("json_path cannot be empty"));
        }
        if path.chars().any(char::is_whitespace) {
            return Err(D::Error::custom("json_path cannot include whitespace"));
        }
        if path.contains('[') || path.contains(']') {
            return Err(D::Error::custom("json_path cannot include array syntax"));
        }
        if path.contains('*') {
            return Err(D::Error::custom("json_path cannot include wildcard syntax"));
        }
        if path.split('.').any(str::is_empty) {
            return Err(D::Error::custom(
                "json_path cannot include empty segments (e.g. '..')",
            ));
        }

        if matches!(
            path.split('.').next(),
            Some("query" | "operationName" | "variables")
        ) {
            return Err(D::Error::custom(
                "json_path cannot access root GraphQL fields: query, operationName, variables",
            ));
        }

        Ok(Self(path))
    }
}

#[derive(Debug, Serialize, JsonSchema, Clone, PartialEq, Eq, Hash)]
#[serde(transparent)]
pub struct PersistedDocumentUrlTemplate(String);

impl PersistedDocumentUrlTemplate {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for PersistedDocumentUrlTemplate {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let template = String::deserialize(deserializer)?;

        validate_url_path_template(&template).map_err(D::Error::custom)?;

        Ok(Self(template))
    }
}

fn validate_url_path_template(template: &str) -> Result<(), String> {
    if template.is_empty() {
        return Err("url_path_param.template cannot be empty".to_string());
    }
    if !template.starts_with('/') {
        return Err("url_path_param.template must start with '/'".to_string());
    }
    if template.contains('?') || template.contains('#') {
        return Err("url_path_param.template cannot include query string or fragment".to_string());
    }

    let raw_segments: Vec<&str> = template.split('/').skip(1).collect();
    if raw_segments.iter().any(|segment| segment.is_empty()) {
        return Err("url_path_param.template cannot include empty segments".to_string());
    }

    let mut id_count = 0;
    for (index, segment) in raw_segments.iter().enumerate() {
        match *segment {
            ":id" => id_count += 1,
            "*" => {}
            "**" => {
                return Err("url_path_param.template does not support '**' segments".to_string());
            }
            literal if literal.starts_with(':') => {
                return Err(format!(
                    "url_path_param.template has unsupported parameter segment '{literal}' at index {index}; only ':id' is allowed"
                ));
            }
            _ => {}
        }
    }

    if id_count != 1 {
        return Err("url_path_param.template must include exactly one ':id' segment".to_string());
    }

    Ok(())
}

#[derive(Debug, Serialize, JsonSchema, Clone, PartialEq, Eq, Hash)]
#[serde(transparent)]
pub struct PersistedDocumentQueryParamName(String);

impl PersistedDocumentQueryParamName {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for PersistedDocumentQueryParamName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let name = String::deserialize(deserializer)?;
        // TODO: improve it
        if name.trim().is_empty() {
            return Err(D::Error::custom("url_query_param.name cannot be empty"));
        }
        Ok(Self(name))
    }
}

impl PersistedDocumentsConfig {
    pub fn default_selectors() -> Vec<PersistedDocumentExtractorConfig> {
        vec![
            PersistedDocumentExtractorConfig::JsonPath {
                path: PersistedDocumentJsonPath("documentId".to_string()),
            },
            PersistedDocumentExtractorConfig::JsonPath {
                path: PersistedDocumentJsonPath("extensions.persistedQuery.sha256Hash".to_string()),
            },
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::{
        PersistedDocumentJsonPath, PersistedDocumentUrlTemplate, PersistedDocumentsConfig,
    };

    #[test]
    fn rejects_root_graphql_fields_for_json_path() {
        for path in ["query", "operationName", "variables", "query.foo"] {
            let raw = format!("\"{path}\"");
            let parsed = serde_json::from_str::<PersistedDocumentJsonPath>(&raw);
            assert!(parsed.is_err(), "expected path '{path}' to be rejected");
        }
    }

    #[test]
    fn allows_non_root_graphql_fields_for_json_path() {
        for path in [
            "documentId",
            "extensions.persistedQuery.sha256Hash",
            "foo.query",
        ] {
            let raw = format!("\"{path}\"");
            let parsed = serde_json::from_str::<PersistedDocumentJsonPath>(&raw);
            assert!(parsed.is_ok(), "expected path '{path}' to be allowed");
        }
    }

    #[test]
    fn enabled_persisted_documents_require_storage() {
        let parsed = serde_json::from_str::<PersistedDocumentsConfig>(
            r#"{
              "enabled": true
            }"#,
        );

        assert!(
            parsed.is_err(),
            "expected storage to be required when enabled"
        );
    }

    #[test]
    fn url_template_rejects_unknown_parameter_segment() {
        let parsed = serde_json::from_str::<PersistedDocumentUrlTemplate>(r#""/p/:docId""#);
        assert!(parsed.is_err(), "expected unknown parameter to be rejected");
    }

    #[test]
    fn url_template_accepts_supported_segment_types() {
        for template in ["/v1/p/:id", "/v1/*/:id", "/v1/*/:id/details"] {
            let raw = format!("\"{template}\"");
            let parsed = serde_json::from_str::<PersistedDocumentUrlTemplate>(&raw);
            assert!(parsed.is_ok(), "expected template '{template}' to be valid");
        }
    }

    #[test]
    fn url_template_rejects_globstar_segment() {
        for template in ["/v1/**/:id", "/:id/**/v2"] {
            let raw = format!("\"{template}\"");
            let parsed = serde_json::from_str::<PersistedDocumentUrlTemplate>(&raw);
            assert!(
                parsed.is_err(),
                "expected template '{template}' to be rejected"
            );
        }
    }
}
