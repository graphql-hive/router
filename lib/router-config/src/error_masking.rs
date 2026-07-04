use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct ErrorMaskingConfig {
    #[serde(default = "default_redacted_error_message")]
    pub redacted_error_message: String,
    #[serde(default)]
    pub all: SubgraphErrorMaskingConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subgraphs: Option<HashMap<String, SubgraphErrorMaskingConfig>>,
}

fn default_redacted_error_message() -> String {
    "Unexpected error".to_string()
}

impl Default for ErrorMaskingConfig {
    fn default() -> Self {
        Self {
            redacted_error_message: default_redacted_error_message(),
            all: SubgraphErrorMaskingConfig::default(),
            subgraphs: None,
        }
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
pub struct SubgraphErrorMaskingConfig {
    #[serde(default = "default_redact_error_message")]
    pub error_message: Option<bool>,
    #[serde(default)]
    pub extensions: Option<ExtensionsMaskingConfig>,
}

fn default_redact_error_message() -> Option<bool> {
    None
}

impl Default for SubgraphErrorMaskingConfig {
    fn default() -> Self {
        Self {
            error_message: Some(true),
            extensions: None,
        }
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields, tag = "mode")]
pub enum ExtensionsMaskingConfig {
    #[serde(rename = "allow")]
    AllowList { keys: Vec<String> },
    #[serde(rename = "deny")]
    DenyList { keys: Vec<String> },
}
