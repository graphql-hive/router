use dashmap::DashMap;
use http::StatusCode;
use serde::Deserialize;
use serde_json::json;
use sonic_rs::{JsonContainerTrait, JsonValueTrait};

use hive_router_plan_executor::{
    executors::http::HttpResponse,
    hooks::on_graphql_params::{OnGraphQLParamsStartHookPayload, OnGraphQLParamsStartHookResult},
    plugin_trait::{EndHookPayload, RouterPlugin, StartHookPayload},
};

#[derive(Deserialize)]
pub struct APQPluginConfig {
    pub enabled: bool,
}

pub struct APQPlugin {
    cache: DashMap<String, String>,
}

#[async_trait::async_trait]
impl RouterPlugin for APQPlugin {
    type Config = APQPluginConfig;
    fn plugin_name() -> &'static str {
        "apq"
    }
    fn from_config(config: Self::Config) -> Option<Self> {
        if config.enabled {
            Some(APQPlugin {
                cache: DashMap::new(),
            })
        } else {
            None
        }
    }
    async fn on_graphql_params<'exec>(
        &'exec self,
        payload: OnGraphQLParamsStartHookPayload<'exec>,
    ) -> OnGraphQLParamsStartHookResult<'exec> {
        payload.on_end(|mut payload| {
            let persisted_query_ext = payload
                .graphql_params
                .extensions
                .as_ref()
                .and_then(|ext| ext.get("persistedQuery"))
                .and_then(|pq| pq.as_object());
            if let Some(persisted_query_ext) = persisted_query_ext {
                match persisted_query_ext.get(&"version").and_then(|v| v.as_i64()) {
                    Some(1) => {}
                    _ => {
                        let body = json!({
                            "errors": [
                                {
                                    "message": "Unsupported persisted query version",
                                    "extensions": {
                                        "code": "UNSUPPORTED_PERSISTED_QUERY_VERSION"
                                    }
                                }
                            ]
                        });
                        return payload.end_response(HttpResponse {
                            body: body.to_string().into_bytes().into(),
                            status: StatusCode::BAD_REQUEST,
                            headers: http::HeaderMap::new(),
                        });
                    }
                }
                let sha256_hash = match persisted_query_ext
                    .get(&"sha256Hash")
                    .and_then(|h| h.as_str())
                {
                    Some(h) => h,
                    None => {
                        let body = json!({
                            "errors": [
                                {
                                    "message": "Missing sha256Hash in persisted query",
                                    "extensions": {
                                        "code": "MISSING_PERSISTED_QUERY_HASH"
                                    }
                                }
                            ]
                        });
                        return payload.end_response(HttpResponse {
                            body: body.to_string().into_bytes().into(),
                            status: StatusCode::BAD_REQUEST,
                            headers: http::HeaderMap::new(),
                        });
                    }
                };
                if let Some(query_param) = &payload.graphql_params.query {
                    // Store the query in the cache
                    self.cache
                        .insert(sha256_hash.to_string(), query_param.to_string());
                } else {
                    // Try to get the query from the cache
                    if let Some(cached_query) = self.cache.get(sha256_hash) {
                        // Update the graphql_params with the cached query
                        payload.graphql_params.query = Some(cached_query.value().to_string());
                    } else {
                        let body = json!({
                            "errors": [
                                {
                                    "message": "PersistedQueryNotFound",
                                    "extensions": {
                                        "code": "PERSISTED_QUERY_NOT_FOUND"
                                    }
                                }
                            ]
                        });
                        return payload.end_response(HttpResponse {
                            body: body.to_string().into_bytes().into(),
                            status: StatusCode::NOT_FOUND,
                            headers: http::HeaderMap::new(),
                        });
                    }
                }
            }

            payload.cont()
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::testkit::{init_router_from_config_inline, wait_for_readiness, SubgraphsServer};

    use hive_router::PluginRegistry;
    use ntex::web::test;
    use serde_json::json;
    #[ntex::test]
    async fn sends_not_found_error_if_query_missing() {
        SubgraphsServer::start().await;
        let app = init_router_from_config_inline(
            r#"
                plugins:
                    apq:
                        enabled: true
            "#,
            Some(PluginRegistry::new().register::<super::APQPlugin>()),
        )
        .await
        .expect("failed to start router");
        wait_for_readiness(&app.app).await;
        let body = json!(
            {
                "extensions": {
                    "persistedQuery": {
                        "version": 1,
                        "sha256Hash": "ecf4edb46db40b5132295c0291d62fb65d6759a9eedfa4d5d612dd5ec54a6b38",
                    },
                },
            }
        );
        let req = test::TestRequest::post()
            .uri("/graphql")
            .header("content-type", "application/json")
            .set_payload(body.to_string());
        let resp = test::call_service(&app.app, req.to_request()).await;
        let body = test::read_body(resp).await;
        let body_json: serde_json::Value =
            serde_json::from_slice(&body).expect("Response body should be valid JSON");
        assert_eq!(
            body_json,
            json!({
                "errors": [
                    {
                        "message": "PersistedQueryNotFound",
                        "extensions": {
                            "code": "PERSISTED_QUERY_NOT_FOUND"
                        }
                    }
                ]
            }),
            "Expected PersistedQueryNotFound error"
        );
    }
    #[ntex::test]
    async fn saves_persisted_query() {
        SubgraphsServer::start().await;
        let app = init_router_from_config_inline(
            r#"
                plugins:
                    apq:
                        enabled: true
            "#,
            Some(PluginRegistry::new().register::<super::APQPlugin>()),
        )
        .await
        .expect("failed to start router");
        wait_for_readiness(&app.app).await;
        let query = "{ users { id } }";
        let sha256_hash = "ecf4edb46db40b5132295c0291d62fb65d6759a9eedfa4d5d612dd5ec54a6b38";
        let body = json!(
            {
                "query": query,
                "extensions": {
                    "persistedQuery": {
                        "version": 1,
                        "sha256Hash": sha256_hash,
                    },
                },
            }
        );
        let req = test::TestRequest::post()
            .uri("/graphql")
            .header("content-type", "application/json")
            .set_payload(body.to_string());
        let resp = test::call_service(&app.app, req.to_request()).await;
        assert!(
            resp.status().is_success(),
            "Expected 200 OK when sending full query"
        );

        // Now send only the hash and expect it to be found
        let body = json!(
            {
                "extensions": {
                    "persistedQuery": {
                        "version": 1,
                        "sha256Hash": sha256_hash,
                    },
                },
            }
        );
        let req = test::TestRequest::post()
            .uri("/graphql")
            .header("content-type", "application/json")
            .set_payload(body.to_string());
        let resp = test::call_service(&app.app, req.to_request()).await;
        assert!(
            resp.status().is_success(),
            "Expected 200 OK when sending persisted query hash"
        );
    }
}
