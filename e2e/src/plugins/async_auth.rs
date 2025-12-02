// From https://github.com/apollographql/router/blob/dev/examples/async-auth/rust/src/allow_client_id_from_file.rs
use serde::Deserialize;
use sonic_rs::json;
use std::path::PathBuf;

use hive_router_plan_executor::{
    executors::http::HttpResponse,
    hooks::on_graphql_params::{OnGraphQLParamsStartHookPayload, OnGraphQLParamsStartHookResult},
    plugin_trait::{RouterPlugin, RouterPluginWithConfig, StartHookPayload},
};

#[derive(Deserialize)]
pub struct AllowClientIdConfig {
    pub enabled: bool,
    pub header: String,
    pub path: String,
}

impl RouterPluginWithConfig for AllowClientIdFromFilePlugin {
    type Config = AllowClientIdConfig;
    fn plugin_name() -> &'static str {
        "allow_client_id_from_file"
    }
    fn from_config(config: AllowClientIdConfig) -> Option<Self> {
        if config.enabled {
            Some(AllowClientIdFromFilePlugin {
                header_key: config.header,
                allowed_ids_path: PathBuf::from(config.path),
            })
        } else {
            None
        }
    }
}

pub struct AllowClientIdFromFilePlugin {
    header_key: String,
    allowed_ids_path: PathBuf,
}

#[async_trait::async_trait]
impl RouterPlugin for AllowClientIdFromFilePlugin {
    // Whenever it is a GraphQL request,
    // We don't use on_http_request here because we want to run this only when it is a GraphQL request
    async fn on_graphql_params<'exec>(
        &'exec self,
        payload: OnGraphQLParamsStartHookPayload<'exec>,
    ) -> OnGraphQLParamsStartHookResult<'exec> {
        let header = payload.router_http_request.headers.get(&self.header_key);
        match header {
            Some(client_id) => {
                let client_id_str = client_id.to_str();
                match client_id_str {
                    Ok(client_id) => {
                        let allowed_clients: Vec<String> = sonic_rs::from_str(
                            std::fs::read_to_string(self.allowed_ids_path.clone())
                                .unwrap()
                                .as_str(),
                        )
                        .unwrap();

                        if !allowed_clients.contains(&client_id.to_string()) {
                            // Prepare an HTTP 403 response with a GraphQL error message
                            let body = json!(
                                {
                                    "errors": [
                                        {
                                            "message": "client-id is not allowed",
                                            "extensions": {
                                                "code": "UNAUTHORIZED_CLIENT_ID"
                                            }
                                        }
                                    ]
                                }
                            );
                            return payload.end_response(HttpResponse {
                                body: sonic_rs::to_vec(&body).unwrap_or_default().into(),
                                headers: http::HeaderMap::new(),
                                status: http::StatusCode::FORBIDDEN,
                            });
                        }
                    }
                    Err(_not_a_string_error) => {
                        let message = format!("'{}' value is not a string", &self.header_key);
                        tracing::error!(message);
                        let body = json!(
                            {
                                "errors": [
                                    {
                                        "message": message,
                                        "extensions": {
                                            "code": "BAD_CLIENT_ID"
                                        }
                                    }
                                ]
                            }
                        );
                        return payload.end_response(HttpResponse {
                            body: sonic_rs::to_vec(&body).unwrap_or_default().into(),
                            headers: http::HeaderMap::new(),
                            status: http::StatusCode::BAD_REQUEST,
                        });
                    }
                }
            }
            None => {
                let message = format!("Missing '{}' header", &self.header_key);
                tracing::error!(message);
                let body = json!(
                    {
                        "errors": [
                            {
                                "message": message,
                                "extensions": {
                                    "code": "AUTH_ERROR"
                                }
                            }
                        ]
                    }
                );
                return payload.end_response(HttpResponse {
                    body: sonic_rs::to_vec(&body).unwrap_or_default().into(),
                    headers: http::HeaderMap::new(),
                    status: http::StatusCode::UNAUTHORIZED,
                });
            }
        }
        payload.cont()
    }
}

#[cfg(test)]
mod tests {
    use crate::testkit::{
        init_graphql_request, init_router_from_config_inline, wait_for_readiness, SubgraphsServer,
    };

    use hive_router::PluginRegistry;
    use ntex::web::test;
    use serde_json::Value;
    #[ntex::test]
    async fn should_allow_only_allowed_client_ids() {
        SubgraphsServer::start().await;

        let app = init_router_from_config_inline(
            r#"
            plugins:
              allow_client_id_from_file:
                enabled: true
                path: "./src/plugins/allowed_clients.json"
                header: "x-client-id"
            "#,
            Some(PluginRegistry::new().register::<super::AllowClientIdFromFilePlugin>()),
        )
        .await
        .expect("Router should initialize successfully");
        wait_for_readiness(&app.app).await;
        // Test with an allowed client id
        let req = init_graphql_request("{ users { id } }", None).header("x-client-id", "urql");
        let resp = test::call_service(&app.app, req.to_request()).await;
        let status = resp.status();
        assert!(status.is_success(), "Expected 200 OK for allowed client id");
        // Test with a disallowed client id
        let req = init_graphql_request("{ users { id } }", None)
            .header("x-client-id", "forbidden-client");
        let resp = test::call_service(&app.app, req.to_request()).await;
        assert_eq!(
            resp.status(),
            http::StatusCode::FORBIDDEN,
            "Expected 403 FORBIDDEN for disallowed client id"
        );
        let body_bytes = test::read_body(resp).await;
        let body_json: Value =
            serde_json::from_slice(&body_bytes).expect("Response body should be valid JSON");
        assert_eq!(
            body_json,
            serde_json::json!({
                "errors": [
                    {
                        "message": "client-id is not allowed",
                        "extensions": {
                            "code": "UNAUTHORIZED_CLIENT_ID"
                        }
                    }
                ]
            }),
            "Expected error message for disallowed client id"
        );
        // Test with missing client id
        let req = init_graphql_request("{ users { id } }", None);
        let resp = test::call_service(&app.app, req.to_request()).await;
        assert_eq!(
            resp.status(),
            http::StatusCode::UNAUTHORIZED,
            "Expected 401 UNAUTHORIZED for missing client id"
        );
        let body_bytes = test::read_body(resp).await;
        let body_json: Value =
            serde_json::from_slice(&body_bytes).expect("Response body should be valid JSON");
        assert_eq!(
            body_json,
            serde_json::json!({
                "errors": [
                    {
                        "message": "Missing 'x-client-id' header",
                        "extensions": {
                            "code": "AUTH_ERROR"
                        }
                    }
                ]
            }),
            "Expected error message for missing client id"
        );
    }
}
