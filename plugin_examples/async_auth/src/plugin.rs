// From https://github.com/apollographql/router/blob/dev/examples/async-auth/rust/src/allow_client_id_from_file.rs
use serde::Deserialize;
use std::path::PathBuf;

use hive_router::{
    async_trait,
    http::StatusCode,
    plugins::{
        hooks::{
            on_graphql_params::{OnGraphQLParamsStartHookPayload, OnGraphQLParamsStartHookResult},
            on_plugin_init::{OnPluginInitPayload, OnPluginInitResult},
        },
        plugin_trait::{RouterPlugin, StartHookPayload},
    },
    sonic_rs, tracing, GraphQLError,
};

#[derive(Deserialize)]
pub struct AllowClientIdConfig {
    pub header: String,
    pub path: String,
}

pub struct AllowClientIdFromFilePlugin {
    header_key: String,
    allowed_ids_path: PathBuf,
}

#[async_trait]
impl RouterPlugin for AllowClientIdFromFilePlugin {
    type Config = AllowClientIdConfig;
    fn plugin_name() -> &'static str {
        "allow_client_id_from_file"
    }
    fn on_plugin_init(payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
        let config = payload.config()?;
        payload.initialize_plugin(Self {
            header_key: config.header,
            allowed_ids_path: PathBuf::from(config.path),
        })
    }
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
                            return payload.end_with_graphql_error(
                                GraphQLError::from_message_and_code(
                                    "client-id is not allowed",
                                    "UNAUTHORIZED_CLIENT_ID",
                                ),
                                StatusCode::FORBIDDEN,
                            );
                        }
                    }
                    Err(_not_a_string_error) => {
                        let message = format!("'{}' value is not a string", &self.header_key);
                        tracing::error!(message);
                        return payload.end_with_graphql_error(
                            GraphQLError::from_message_and_code(message, "BAD_CLIENT_ID"),
                            StatusCode::BAD_REQUEST,
                        );
                    }
                }
            }
            None => {
                let message = format!("Missing '{}' header", &self.header_key);
                tracing::error!(message);
                return payload.end_with_graphql_error(
                    GraphQLError::from_message_and_code(message, "AUTH_ERROR"),
                    StatusCode::UNAUTHORIZED,
                );
            }
        }
        payload.proceed()
    }
}
