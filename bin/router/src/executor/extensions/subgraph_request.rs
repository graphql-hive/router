use std::collections::HashMap;

use ahash::HashSet;
use sonic_rs::Value;

use crate::config::jwt_auth::JwtAuthConfig;
use crate::config::persisted_documents::{
    PersistedDocumentExtractorConfig, PersistedDocumentsConfig,
};
use crate::config::request_extensions::{RequestExtensionsConfig, RequestExtensionsPropagateMode};
use crate::executor::executors::common::SubgraphRequestExtensions;
use crate::executor::executors::http_callback::CALLBACK_EXTENSIONS_SUBSCRIPTION_KEY;
use crate::pipeline::persisted_documents::extract::extractors::apollo::APOLLO_EXTENSIONS_PERSISTED_QUERY_KEY;
use crate::pipeline::websocket_server::WEBSOCKET_EXTENSIONS_HEADERS_KEY;

/// Top-level keys of the client request `extensions` that are always owned by the router,
/// paired with what owns them.
const ROUTER_OWNED_KEYS: [(&str, &str); 3] = [
    (
        APOLLO_EXTENSIONS_PERSISTED_QUERY_KEY,
        "automatic persisted queries",
    ),
    (
        WEBSOCKET_EXTENSIONS_HEADERS_KEY,
        "websocket operation headers",
    ),
    (
        CALLBACK_EXTENSIONS_SUBSCRIPTION_KEY,
        "HTTP callback subscriptions",
    ),
];

/// Decides which client request `extensions` are sent to subgraphs.
pub enum SubgraphRequestExtensionsPlan {
    /// Only these keys are propagated.
    Allow(HashSet<String>),
    /// Every key except these is propagated. Includes the router-owned keys.
    Deny(HashSet<String>),
}

impl SubgraphRequestExtensionsPlan {
    /// Selects the client request extensions to send to subgraphs.
    /// Returns `None` when nothing is left to send.
    pub fn select(
        &self,
        client_extensions: &HashMap<String, Value>,
    ) -> Option<SubgraphRequestExtensions> {
        let selected: SubgraphRequestExtensions = client_extensions
            .iter()
            .filter(|(key, _)| match self {
                Self::Allow(keys) => keys.contains(key.as_str()),
                Self::Deny(keys) => !keys.contains(key.as_str()),
            })
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();

        (!selected.is_empty()).then_some(selected)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SubgraphRequestExtensionsPlanError {
    #[error("'propagate.keys' can't be empty in 'allow' mode")]
    EmptyAllowList,
    #[error("'propagate.keys' can't allow '{key}', it's owned by the router ({owner})")]
    RouterOwnedKey { key: String, owner: String },
}

pub fn compile_subgraph_request_extensions_plan(
    cfg: &RequestExtensionsConfig,
    jwt: &JwtAuthConfig,
    persisted_documents: &PersistedDocumentsConfig,
) -> Result<Option<SubgraphRequestExtensionsPlan>, SubgraphRequestExtensionsPlanError> {
    let Some(propagate) = cfg.propagate.as_ref() else {
        return Ok(None);
    };

    let router_owned = router_owned_keys(jwt, persisted_documents);

    let plan = match propagate.mode {
        RequestExtensionsPropagateMode::Allow => {
            if propagate.keys.is_empty() {
                return Err(SubgraphRequestExtensionsPlanError::EmptyAllowList);
            }
            if let Some((key, owner)) = propagate
                .keys
                .iter()
                .find_map(|key| router_owned.get_key_value(key.as_str()))
            {
                return Err(SubgraphRequestExtensionsPlanError::RouterOwnedKey {
                    key: key.clone(),
                    owner: owner.clone(),
                });
            }
            SubgraphRequestExtensionsPlan::Allow(propagate.keys.iter().cloned().collect())
        }
        RequestExtensionsPropagateMode::Deny => SubgraphRequestExtensionsPlan::Deny(
            propagate
                .keys
                .iter()
                .cloned()
                .chain(router_owned.into_keys())
                .collect(),
        ),
    };

    Ok(Some(plan))
}

/// Collects the top-level extension keys the router reads or writes, paired with what owns them.
fn router_owned_keys(
    jwt: &JwtAuthConfig,
    persisted_documents: &PersistedDocumentsConfig,
) -> HashMap<String, String> {
    let mut keys: HashMap<String, String> = ROUTER_OWNED_KEYS
        .iter()
        .map(|(key, owner)| (key.to_string(), owner.to_string()))
        .collect();

    if jwt.is_jwt_extensions_forwarding_enabled() {
        keys.insert(
            jwt.forward_claims_to_upstream_extensions.field_name.clone(),
            "jwt.forward_claims_to_upstream_extensions".to_string(),
        );
    }

    if persisted_documents.enabled {
        let selectors = persisted_documents
            .selectors
            .clone()
            .unwrap_or_else(PersistedDocumentsConfig::default_selectors);
        for selector in selectors {
            if let PersistedDocumentExtractorConfig::JsonPath { path } = selector {
                let mut segments = path.as_str().split('.');
                if let (Some("extensions"), Some(key)) = (segments.next(), segments.next()) {
                    keys.insert(
                        key.to_string(),
                        format!("persisted_documents selector '{}'", path.as_str()),
                    );
                }
            }
        }
    }

    keys
}

#[cfg(test)]
mod tests {
    use sonic_rs::json;

    use super::*;
    use crate::config::persisted_documents::PersistedDocumentJsonPath;
    use crate::config::request_extensions::RequestExtensionsPropagateConfig;

    fn config(mode: RequestExtensionsPropagateMode, keys: &[&str]) -> RequestExtensionsConfig {
        RequestExtensionsConfig {
            propagate: Some(RequestExtensionsPropagateConfig {
                mode,
                keys: keys.iter().map(|key| key.to_string()).collect(),
            }),
        }
    }

    fn compile(cfg: &RequestExtensionsConfig) -> SubgraphRequestExtensionsPlan {
        compile_subgraph_request_extensions_plan(
            cfg,
            &JwtAuthConfig::default(),
            &PersistedDocumentsConfig::default(),
        )
        .expect("config should be valid")
        .expect("plan should be compiled")
    }

    fn jwt_forwarding(field_name: &str) -> JwtAuthConfig {
        let mut jwt = JwtAuthConfig {
            enabled: true,
            ..Default::default()
        };
        jwt.forward_claims_to_upstream_extensions.enabled = true;
        jwt.forward_claims_to_upstream_extensions.field_name = field_name.to_string();
        jwt
    }

    fn persisted_documents(paths: &[&str]) -> PersistedDocumentsConfig {
        PersistedDocumentsConfig {
            enabled: true,
            selectors: Some(
                paths
                    .iter()
                    .map(|path| PersistedDocumentExtractorConfig::JsonPath {
                        path: serde_json::from_value::<PersistedDocumentJsonPath>(
                            serde_json::Value::String(path.to_string()),
                        )
                        .expect("valid json path"),
                    })
                    .collect(),
            ),
            ..Default::default()
        }
    }

    fn client_extensions(value: Value) -> HashMap<String, Value> {
        sonic_rs::from_str(&value.to_string()).expect("extensions should be an object")
    }

    fn selected_keys(extensions: Option<SubgraphRequestExtensions>) -> Vec<String> {
        let mut keys: Vec<String> = extensions.unwrap_or_default().into_keys().collect();
        keys.sort();
        keys
    }

    #[test]
    fn no_plan_without_propagate() {
        let plan = compile_subgraph_request_extensions_plan(
            &RequestExtensionsConfig::default(),
            &JwtAuthConfig::default(),
            &PersistedDocumentsConfig::default(),
        )
        .expect("config should be valid");
        assert!(plan.is_none());
    }

    #[test]
    fn allow_mode_selects_listed_keys() {
        let plan = compile(&config(
            RequestExtensionsPropagateMode::Allow,
            &["tenant", "clientInfo"],
        ));
        let extensions = client_extensions(json!({
            "tenant": "acme",
            "clientInfo": { "name": "web" },
            "debug": true,
        }));

        let selected = plan.select(&extensions).expect("should select keys");
        assert_eq!(selected.len(), 2);
        assert_eq!(selected["tenant"], json!("acme"));
        assert_eq!(selected["clientInfo"], json!({ "name": "web" }));
    }

    #[test]
    fn deny_mode_drops_listed_and_router_owned_keys() {
        let plan = compile(&config(RequestExtensionsPropagateMode::Deny, &["debug"]));
        let extensions = client_extensions(json!({
            "tenant": "acme",
            "debug": true,
            "persistedQuery": { "version": 1, "sha256Hash": "abc" },
            "headers": { "authorization": "Bearer secret" },
            "subscription": { "callbackUrl": "http://attacker" },
        }));

        assert_eq!(selected_keys(plan.select(&extensions)), vec!["tenant"]);
    }

    #[test]
    fn deny_mode_with_no_keys_selects_everything_not_router_owned() {
        let plan = compile(&config(RequestExtensionsPropagateMode::Deny, &[]));
        let extensions = client_extensions(json!({
            "tenant": "acme",
            "debug": true,
            "persistedQuery": { "version": 1, "sha256Hash": "abc" },
        }));

        assert_eq!(
            selected_keys(plan.select(&extensions)),
            vec!["debug", "tenant"]
        );
    }

    #[test]
    fn selects_nothing_when_no_key_matches() {
        let plan = compile(&config(RequestExtensionsPropagateMode::Allow, &["tenant"]));
        let extensions = client_extensions(json!({ "debug": true }));

        assert!(plan.select(&extensions).is_none());
    }

    #[test]
    fn deny_mode_drops_jwt_forwarding_field() {
        let plan = compile_subgraph_request_extensions_plan(
            &config(RequestExtensionsPropagateMode::Deny, &[]),
            &jwt_forwarding("claims"),
            &PersistedDocumentsConfig::default(),
        )
        .expect("config should be valid")
        .expect("plan should be compiled");
        let extensions = client_extensions(json!({
            "tenant": "acme",
            "claims": { "role": "admin" },
        }));

        assert_eq!(selected_keys(plan.select(&extensions)), vec!["tenant"]);
    }

    #[test]
    fn jwt_field_is_not_router_owned_when_forwarding_is_disabled() {
        let plan = compile(&config(RequestExtensionsPropagateMode::Allow, &["jwt"]));
        let extensions = client_extensions(json!({ "jwt": "value" }));

        assert_eq!(selected_keys(plan.select(&extensions)), vec!["jwt"]);
    }

    #[test]
    fn deny_mode_drops_persisted_document_extension_keys() {
        let plan = compile_subgraph_request_extensions_plan(
            &config(RequestExtensionsPropagateMode::Deny, &[]),
            &JwtAuthConfig::default(),
            &persisted_documents(&["documentId", "extensions.docs.id"]),
        )
        .expect("config should be valid")
        .expect("plan should be compiled");
        let extensions = client_extensions(json!({
            "tenant": "acme",
            "docs": { "id": "123" },
        }));

        assert_eq!(selected_keys(plan.select(&extensions)), vec!["tenant"]);
    }

    #[test]
    fn rejects_empty_allow_list() {
        let result = compile_subgraph_request_extensions_plan(
            &config(RequestExtensionsPropagateMode::Allow, &[]),
            &JwtAuthConfig::default(),
            &PersistedDocumentsConfig::default(),
        );

        assert!(matches!(
            result,
            Err(SubgraphRequestExtensionsPlanError::EmptyAllowList)
        ));
    }

    #[test]
    fn rejects_allowing_router_owned_keys() {
        let cases = [
            (
                "persistedQuery",
                JwtAuthConfig::default(),
                PersistedDocumentsConfig::default(),
            ),
            (
                "claims",
                jwt_forwarding("claims"),
                PersistedDocumentsConfig::default(),
            ),
            (
                "docs",
                JwtAuthConfig::default(),
                persisted_documents(&["extensions.docs.id"]),
            ),
        ];

        for (key, jwt, persisted_documents) in cases {
            let result = compile_subgraph_request_extensions_plan(
                &config(RequestExtensionsPropagateMode::Allow, &["tenant", key]),
                &jwt,
                &persisted_documents,
            );

            match result {
                Err(SubgraphRequestExtensionsPlanError::RouterOwnedKey { key: owned, .. }) => {
                    assert_eq!(owned, key)
                }
                _ => panic!("expected '{key}' to be rejected as a router-owned key"),
            }
        }
    }
}
