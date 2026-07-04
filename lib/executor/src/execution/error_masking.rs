use std::collections::{HashMap, HashSet};

use crate::response::graphql_error::{GraphQLError, GraphQLErrorExtensions};
use hive_router_config::error_masking::{
    ErrorMaskingConfig, ExtensionsMaskingConfig, SubgraphErrorMaskingConfig,
};

pub struct ErrorMaskingRuntime {
    default_redacted_error_message: String,
    per_subgraph_config: HashMap<String, ErrorMaskingCompiledConfig>,
    default_config: ErrorMaskingCompiledConfig,
}

impl ErrorMaskingRuntime {
    pub fn apply(&self, err: &mut GraphQLError) {
        if let Some(service_name) = &err.extensions.service_name {
            let effective_config = self
                .per_subgraph_config
                .get(service_name)
                .unwrap_or(&self.default_config);

            if effective_config.redact_error_message {
                err.message = self.default_redacted_error_message.clone();
            }

            if let Some(plan) = &effective_config.extensions_plan {
                plan.apply(&mut err.extensions);
            }
        }
    }

    pub fn compile_from_config(config: &ErrorMaskingConfig) -> Self {
        Self {
            default_redacted_error_message: config.redacted_error_message.clone(),
            per_subgraph_config: config
                .subgraphs
                .as_ref()
                .map(|subgraphs| {
                    subgraphs
                        .iter()
                        .map(|(name, cfg)| (name.clone(), Self::compile_config(cfg)))
                        .collect()
                })
                .unwrap_or_default(),
            default_config: Self::compile_config(&config.all),
        }
    }

    fn compile_config(config: &SubgraphErrorMaskingConfig) -> ErrorMaskingCompiledConfig {
        let extensions_plan = config.extensions.as_ref().map(|cfg| match cfg {
            ExtensionsMaskingConfig::AllowList { keys } => {
                RedactExtensionsPlan::Allow(keys.clone())
            }
            ExtensionsMaskingConfig::DenyList { keys } => RedactExtensionsPlan::Deny(keys.clone()),
        });

        ErrorMaskingCompiledConfig {
            redact_error_message: config.error_message.unwrap_or(true),
            extensions_plan,
        }
    }
}

struct ErrorMaskingCompiledConfig {
    redact_error_message: bool,
    extensions_plan: Option<RedactExtensionsPlan>,
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
            Self::remove_field(extensions, &removal_path);
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
                "serviceName" => allow_service_name = true,
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
                return;
            }
            "serviceName" => {
                extensions.service_name = None;
                return;
            }
            "affectedPath" => {
                extensions.affected_path = None;
                return;
            }
            _ => {
                extensions.extensions.remove(key);
            }
        }
    }
}
