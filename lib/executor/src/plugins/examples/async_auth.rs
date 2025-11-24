use std::path::PathBuf;

// From https://github.com/apollographql/router/blob/dev/examples/async-auth/rust/src/allow_client_id_from_file.rs
use serde::Deserialize;
use sonic_rs::json;

use crate::{
    execution::plan::PlanExecutionOutput,
    hooks::on_graphql_params::{OnGraphQLParamsEndPayload, OnGraphQLParamsStartPayload},
    plugin_trait::{HookResult, RouterPlugin, RouterPluginWithConfig, StartPayload},
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
        payload: OnGraphQLParamsStartPayload<'exec>,
    ) -> HookResult<'exec, OnGraphQLParamsStartPayload<'exec>, OnGraphQLParamsEndPayload> {
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
                            return payload.end_response(PlanExecutionOutput {
                                body: sonic_rs::to_vec(&body).unwrap_or_default(),
                                headers: http::HeaderMap::new(),
                                status: http::StatusCode::FORBIDDEN,
                            });
                        }
                    }
                    Err(_not_a_string_error) => {
                        let message = format!("'{}' value is not a string", self.header_key);
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
                        return payload.end_response(PlanExecutionOutput {
                            body: sonic_rs::to_vec(&body).unwrap_or_default(),
                            headers: http::HeaderMap::new(),
                            status: http::StatusCode::BAD_REQUEST,
                        });
                    }
                }
            }
            None => {
                let message = format!("Missing '{}' header", self.header_key);
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
                return payload.end_response(PlanExecutionOutput {
                    body: sonic_rs::to_vec(&body).unwrap_or_default(),
                    headers: http::HeaderMap::new(),
                    status: http::StatusCode::UNAUTHORIZED,
                });
            }
        }
        payload.cont()
    }
}
