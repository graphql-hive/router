use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct ErrorMaskingConfig {
    /// A switch for enabling or disabling error masking feature completely.
    ///
    /// Defaults to `true`.
    ///
    /// You can also disable it by setting the `DISABLE_SUBGRAPH_ERROR_MASKING=true` environment variable.
    #[serde(default = "default_feature_enabled")]
    pub enabled: bool,
    /// The error message to redact in subgraph errors. The default is "Unexpected error".
    #[serde(default = "default_redacted_error_message")]
    pub redacted_error_message: String,
    /// The default error masking configuration for all subgraphs.
    #[serde(default)]
    pub all: AllErrorMaskingConfig,
    /// The error masking configuration for individual subgraphs.
    /// Any configuration field that will be specified here, will override the configuration in `all`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subgraphs: Option<HashMap<String, SubgraphErrorMaskingConfig>>,
}

fn default_feature_enabled() -> bool {
    true
}

fn default_redacted_error_message() -> String {
    "Unexpected error".to_string()
}

impl Default for ErrorMaskingConfig {
    fn default() -> Self {
        Self {
            redacted_error_message: default_redacted_error_message(),
            all: AllErrorMaskingConfig::default(),
            subgraphs: None,
            enabled: default_feature_enabled(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct AllErrorMaskingConfig {
    /// Whether to redact the error message in subgraph errors. The default is `true`.
    #[serde(default = "default_redact_error_message")]
    pub enabled: bool,
    /// Whether to redact the `extensions` in errors.
    ///
    /// You may pick the execution mode by setting `mode: allow` or `mode: deny`.
    /// Note: only root-level fields are supported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extensions: Option<ExtensionsMaskingConfig>,
}

fn default_redact_error_message() -> bool {
    true
}

impl Default for AllErrorMaskingConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            extensions: None,
        }
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct SubgraphErrorMaskingConfig {
    /// Whether to redact the `error_message` in errors, for that specific subgraph.
    ///
    /// Configuring this will override the global `all.error_message` setting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// Whether to redact the `extensions` in errors, for that specific subgraph.
    /// Configuring this will override the global `all.extensions` setting.
    ///
    /// You may pick the execution mode by setting `mode: allow` or `mode: deny`.
    /// Note: only root-level fields are supported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extensions: Option<ExtensionsMaskingConfig>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields, tag = "mode")]
pub enum ExtensionsMaskingConfig {
    /// Redact extensions based on the allowlist.
    #[serde(rename = "allow")]
    AllowList { keys: Vec<String> },
    /// Redact extensions based on the denylist.
    #[serde(rename = "deny")]
    DenyList { keys: Vec<String> },
}
