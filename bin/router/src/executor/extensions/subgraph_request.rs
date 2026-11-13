use std::collections::HashMap;

use ahash::{HashMap as AHashMap, HashSet};
use sonic_rs::Value;

use crate::config::jwt_auth::JwtAuthConfig;
use crate::config::persisted_documents::{
    PersistedDocumentExtractorConfig, PersistedDocumentsConfig,
};
use crate::config::request_extensions::{
    RequestExtensionsConfig, RequestExtensionsPropagateMode, RequestExtensionsPropagateRule,
};
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

/// Decides which client request `extensions` are sent to a subgraph.
enum PropagateRule {
    /// Only these keys are propagated.
    Allow(HashSet<String>),
    /// Every key except these is propagated. Includes the router-owned keys.
    Deny(HashSet<String>),
}

impl PropagateRule {
    /// Returns `None` when nothing is left to send.
    fn select(
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

/// Decides which client request `extensions` are sent to each subgraph.
pub struct SubgraphRequestExtensionsPlan {
    /// The rule applied to all subgraphs.
    all: Option<PropagateRule>,
    /// The rules of individual subgraphs, replacing the `all` rule.
    /// `None` when the subgraph receives no client request extensions.
    by_subgraph: AHashMap<String, Option<PropagateRule>>,
}

impl SubgraphRequestExtensionsPlan {
    /// Selects the client request extensions to send to each subgraph.
    /// Returns `None` when nothing is left to send to any subgraph.
    pub fn select(
        &self,
        client_extensions: &HashMap<String, Value>,
    ) -> Option<SelectedSubgraphRequestExtensions> {
        let all = self
            .all
            .as_ref()
            .and_then(|rule| rule.select(client_extensions));
        let by_subgraph: AHashMap<String, Option<SubgraphRequestExtensions>> = self
            .by_subgraph
            .iter()
            .map(|(subgraph_name, rule)| {
                (
                    subgraph_name.clone(),
                    rule.as_ref()
                        .and_then(|rule| rule.select(client_extensions)),
                )
            })
            .collect();

        if all.is_none() && by_subgraph.values().all(Option::is_none) {
            return None;
        }

        Some(SelectedSubgraphRequestExtensions { all, by_subgraph })
    }
}

/// The client request `extensions` selected for each subgraph of a single request.
pub struct SelectedSubgraphRequestExtensions {
    all: Option<SubgraphRequestExtensions>,
    by_subgraph: AHashMap<String, Option<SubgraphRequestExtensions>>,
}

impl SelectedSubgraphRequestExtensions {
    /// The extensions to send to the given subgraph, if any.
    pub fn for_subgraph(&self, subgraph_name: &str) -> Option<&SubgraphRequestExtensions> {
        match self.by_subgraph.get(subgraph_name) {
            Some(selected) => selected.as_ref(),
            None => self.all.as_ref(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SubgraphRequestExtensionsPlanError {
    #[error("'{scope}.keys' can't be empty in 'allow' mode")]
    EmptyAllowList { scope: String },
    #[error("'{scope}.keys' can't allow '{key}', it's owned by the router ({owner})")]
    RouterOwnedKey {
        scope: String,
        key: String,
        owner: String,
    },
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

    let all = propagate
        .all
        .as_ref()
        .map(|rule| compile_propagate_rule(rule, "propagate.all", &router_owned))
        .transpose()?;
    let by_subgraph = propagate
        .subgraphs
        .iter()
        .map(|(subgraph_name, rule)| {
            let rule = rule
                .as_ref()
                .map(|rule| {
                    compile_propagate_rule(
                        rule,
                        &format!("propagate.subgraphs.{subgraph_name}"),
                        &router_owned,
                    )
                })
                .transpose()?;
            Ok((subgraph_name.clone(), rule))
        })
        .collect::<Result<AHashMap<_, _>, _>>()?;

    if all.is_none() && by_subgraph.values().all(Option::is_none) {
        return Ok(None);
    }

    Ok(Some(SubgraphRequestExtensionsPlan { all, by_subgraph }))
}

fn compile_propagate_rule(
    rule: &RequestExtensionsPropagateRule,
    scope: &str,
    router_owned: &HashMap<String, String>,
) -> Result<PropagateRule, SubgraphRequestExtensionsPlanError> {
    match rule.mode {
        RequestExtensionsPropagateMode::Allow => {
            if rule.keys.is_empty() {
                return Err(SubgraphRequestExtensionsPlanError::EmptyAllowList {
                    scope: scope.to_string(),
                });
            }
            if let Some((key, owner)) = rule
                .keys
                .iter()
                .find_map(|key| router_owned.get_key_value(key.as_str()))
            {
                return Err(SubgraphRequestExtensionsPlanError::RouterOwnedKey {
                    scope: scope.to_string(),
                    key: key.clone(),
                    owner: owner.clone(),
                });
            }
            Ok(PropagateRule::Allow(rule.keys.iter().cloned().collect()))
        }
        RequestExtensionsPropagateMode::Deny => Ok(PropagateRule::Deny(
            rule.keys
                .iter()
                .chain(router_owned.keys())
                .cloned()
                .collect(),
        )),
    }
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

    fn rule(mode: RequestExtensionsPropagateMode, keys: &[&str]) -> RequestExtensionsPropagateRule {
        RequestExtensionsPropagateRule {
            mode,
            keys: keys.iter().map(|key| key.to_string()).collect(),
        }
    }

    fn propagate(
        all: Option<RequestExtensionsPropagateRule>,
        subgraphs: &[(&str, Option<RequestExtensionsPropagateRule>)],
    ) -> RequestExtensionsConfig {
        RequestExtensionsConfig {
            propagate: Some(RequestExtensionsPropagateConfig {
                all,
                subgraphs: subgraphs
                    .iter()
                    .map(|(name, rule)| (name.to_string(), rule.clone()))
                    .collect(),
            }),
        }
    }

    fn config(mode: RequestExtensionsPropagateMode, keys: &[&str]) -> RequestExtensionsConfig {
        propagate(Some(rule(mode, keys)), &[])
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

    /// The sorted keys selected for the given subgraph.
    fn selected_keys(
        plan: &SubgraphRequestExtensionsPlan,
        extensions: &HashMap<String, Value>,
        subgraph_name: &str,
    ) -> Vec<String> {
        let selected = plan.select(extensions);
        let mut keys: Vec<String> = selected
            .as_ref()
            .and_then(|selected| selected.for_subgraph(subgraph_name))
            .map(|extensions| extensions.keys().cloned().collect())
            .unwrap_or_default();
        keys.sort();
        keys
    }

    #[test]
    fn no_plan_without_propagate() {
        let configs = [
            RequestExtensionsConfig::default(),
            propagate(None, &[]),
            propagate(None, &[("reviews", None)]),
        ];

        for cfg in configs {
            let plan = compile_subgraph_request_extensions_plan(
                &cfg,
                &JwtAuthConfig::default(),
                &PersistedDocumentsConfig::default(),
            )
            .expect("config should be valid");
            assert!(plan.is_none());
        }
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
        let selected = selected
            .for_subgraph("products")
            .expect("should select keys for any subgraph");
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

        assert_eq!(
            selected_keys(&plan, &extensions, "products"),
            vec!["tenant"]
        );
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
            selected_keys(&plan, &extensions, "products"),
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
    fn subgraph_rule_replaces_all_rule() {
        let plan = compile(&propagate(
            Some(rule(RequestExtensionsPropagateMode::Allow, &["tenant"])),
            &[(
                "reviews",
                Some(rule(RequestExtensionsPropagateMode::Allow, &["clientInfo"])),
            )],
        ));
        let extensions = client_extensions(json!({
            "tenant": "acme",
            "clientInfo": { "name": "web" },
        }));

        assert_eq!(
            selected_keys(&plan, &extensions, "products"),
            vec!["tenant"]
        );
        assert_eq!(
            selected_keys(&plan, &extensions, "reviews"),
            vec!["clientInfo"]
        );
    }

    #[test]
    fn subgraph_set_to_null_receives_nothing() {
        let plan = compile(&propagate(
            Some(rule(RequestExtensionsPropagateMode::Deny, &[])),
            &[("reviews", None)],
        ));
        let extensions = client_extensions(json!({ "tenant": "acme" }));

        assert_eq!(
            selected_keys(&plan, &extensions, "products"),
            vec!["tenant"]
        );
        assert!(selected_keys(&plan, &extensions, "reviews").is_empty());
    }

    #[test]
    fn subgraph_rule_without_all_rule() {
        let plan = compile(&propagate(
            None,
            &[(
                "reviews",
                Some(rule(RequestExtensionsPropagateMode::Deny, &[])),
            )],
        ));
        let extensions = client_extensions(json!({
            "tenant": "acme",
            "persistedQuery": { "version": 1, "sha256Hash": "abc" },
        }));

        assert_eq!(selected_keys(&plan, &extensions, "reviews"), vec!["tenant"]);
        assert!(selected_keys(&plan, &extensions, "products").is_empty());
    }

    #[test]
    fn selects_nothing_when_no_subgraph_matches() {
        let plan = compile(&propagate(
            Some(rule(RequestExtensionsPropagateMode::Allow, &["tenant"])),
            &[(
                "reviews",
                Some(rule(RequestExtensionsPropagateMode::Allow, &["clientInfo"])),
            )],
        ));
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

        assert_eq!(
            selected_keys(&plan, &extensions, "products"),
            vec!["tenant"]
        );
    }

    #[test]
    fn jwt_field_is_not_router_owned_when_forwarding_is_disabled() {
        let plan = compile(&config(RequestExtensionsPropagateMode::Allow, &["jwt"]));
        let extensions = client_extensions(json!({ "jwt": "value" }));

        assert_eq!(selected_keys(&plan, &extensions, "products"), vec!["jwt"]);
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

        assert_eq!(
            selected_keys(&plan, &extensions, "products"),
            vec!["tenant"]
        );
    }

    #[test]
    fn rejects_empty_allow_list() {
        let cases = [
            (
                "propagate.all",
                config(RequestExtensionsPropagateMode::Allow, &[]),
            ),
            (
                "propagate.subgraphs.reviews",
                propagate(
                    None,
                    &[(
                        "reviews",
                        Some(rule(RequestExtensionsPropagateMode::Allow, &[])),
                    )],
                ),
            ),
        ];

        for (expected_scope, cfg) in cases {
            let result = compile_subgraph_request_extensions_plan(
                &cfg,
                &JwtAuthConfig::default(),
                &PersistedDocumentsConfig::default(),
            );

            match result {
                Err(SubgraphRequestExtensionsPlanError::EmptyAllowList { scope }) => {
                    assert_eq!(scope, expected_scope)
                }
                _ => panic!("expected an empty allow list in '{expected_scope}' to be rejected"),
            }
        }
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
                Err(SubgraphRequestExtensionsPlanError::RouterOwnedKey {
                    scope,
                    key: owned,
                    ..
                }) => {
                    assert_eq!(scope, "propagate.all");
                    assert_eq!(owned, key);
                }
                _ => panic!("expected '{key}' to be rejected as a router-owned key"),
            }
        }
    }

    #[test]
    fn rejects_allowing_router_owned_keys_in_subgraph_scope() {
        let result = compile_subgraph_request_extensions_plan(
            &propagate(
                Some(rule(RequestExtensionsPropagateMode::Allow, &["tenant"])),
                &[(
                    "reviews",
                    Some(rule(RequestExtensionsPropagateMode::Allow, &["headers"])),
                )],
            ),
            &JwtAuthConfig::default(),
            &PersistedDocumentsConfig::default(),
        );

        match result {
            Err(SubgraphRequestExtensionsPlanError::RouterOwnedKey { scope, key, .. }) => {
                assert_eq!(scope, "propagate.subgraphs.reviews");
                assert_eq!(key, "headers");
            }
            _ => panic!("expected 'headers' to be rejected as a router-owned key"),
        }
    }
}
