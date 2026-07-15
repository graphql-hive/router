use std::collections::{HashMap, HashSet};

use crate::response::graphql_error::{GraphQLError, GraphQLErrorExtensions};
use hive_router_config::error_masking::{ErrorMaskingConfig, ExtensionsMaskingConfig};

pub struct ErrorMaskingRuntime {
    redacted_error_message: String,

    // Error masking for the error message, split by "all" and subgraph-specific
    default_error_masking: bool,
    per_subgraph_error_masking: HashMap<String, bool>,

    // Error masking for the error extensions, split by "all" and subgraph-specific
    default_extensions_masking: Option<RedactExtensionsPlan>,
    per_subgraph_extensions_masking: HashMap<String, Option<RedactExtensionsPlan>>,
}

impl ErrorMaskingRuntime {
    pub fn apply(&self, err: &mut GraphQLError) {
        if let Some(service_name) = &err.extensions.service_name {
            let should_mask_message = self
                .per_subgraph_error_masking
                .get(service_name)
                .copied()
                .unwrap_or(self.default_error_masking);

            if should_mask_message {
                err.message = self.redacted_error_message.clone();
            }

            let extensions_masking_config = self
                .per_subgraph_extensions_masking
                .get(service_name)
                .and_then(|plan| plan.as_ref())
                .or(self.default_extensions_masking.as_ref());

            if let Some(plan) = extensions_masking_config {
                plan.apply(&mut err.extensions);
            }
        }
    }

    pub fn compile_from_config(config: &ErrorMaskingConfig) -> Self {
        Self {
            redacted_error_message: config.redacted_error_message.clone(),
            default_error_masking: config.all.error_message,
            per_subgraph_error_masking: config
                .subgraphs
                .as_ref()
                .map(|subgraphs| {
                    subgraphs
                        .iter()
                        .map(|(name, cfg)| {
                            (
                                name.clone(),
                                cfg.error_message.unwrap_or(config.all.error_message),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default(),
            default_extensions_masking: config
                .all
                .extensions
                .as_ref()
                .map(Self::compile_extensions_config),
            per_subgraph_extensions_masking: config
                .subgraphs
                .as_ref()
                .map(|subgraphs| {
                    subgraphs
                        .iter()
                        .map(|(name, cfg)| {
                            (
                                name.clone(),
                                cfg.extensions.as_ref().map(Self::compile_extensions_config),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    fn compile_extensions_config(
        extensions_config: &ExtensionsMaskingConfig,
    ) -> RedactExtensionsPlan {
        match extensions_config {
            ExtensionsMaskingConfig::AllowList { keys } => {
                RedactExtensionsPlan::Allow(keys.clone())
            }
            ExtensionsMaskingConfig::DenyList { keys } => RedactExtensionsPlan::Deny(keys.clone()),
        }
    }
}

enum RedactExtensionsPlan {
    Allow(Vec<String>),
    Deny(Vec<String>),
}

impl RedactExtensionsPlan {
    pub fn apply(&self, extensions: &mut GraphQLErrorExtensions) {
        match self {
            RedactExtensionsPlan::Allow(list) => {
                Self::apply_allow_list(extensions, list);
            }
            RedactExtensionsPlan::Deny(list) => {
                Self::apply_deny_list(extensions, list);
            }
        }
    }

    fn apply_deny_list(extensions: &mut GraphQLErrorExtensions, list: &[String]) {
        for removal_path in list {
            Self::remove_field(extensions, removal_path);
        }
    }

    fn apply_allow_list(extensions: &mut GraphQLErrorExtensions, list: &[String]) {
        let mut allow_code = false;
        let mut allow_service_name = false;
        let mut allow_affected_path = false;
        let mut allowed_keys = HashSet::new();

        for key in list {
            match key.as_str() {
                "code" => allow_code = true,
                "service" => allow_service_name = true,
                "affectedPath" => allow_affected_path = true,
                other => {
                    allowed_keys.insert(other);
                }
            }
        }

        if !allow_code {
            extensions.code = None;
        }

        if !allow_service_name {
            extensions.service_name = None;
        }

        if !allow_affected_path {
            extensions.affected_path = None;
        }

        extensions
            .extensions
            .retain(|key, _| allowed_keys.contains(key.as_str()));
    }

    fn remove_field(extensions: &mut GraphQLErrorExtensions, key: &str) {
        match key {
            "code" => {
                extensions.code = None;
            }
            "service" => {
                extensions.service_name = None;
            }
            "affectedPath" => {
                extensions.affected_path = None;
            }
            _ => {
                extensions.extensions.remove(key);
            }
        }
    }
}
