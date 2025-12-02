// Same with https://github.com/apollographql/router/blob/dev/examples/forbid-anonymous-operations/rust/src/forbid_anonymous_operations.rs

use http::StatusCode;
use serde::Deserialize;
use sonic_rs::json;

use hive_router_plan_executor::{
    executors::http::HttpResponse,
    hooks::on_graphql_params::{OnGraphQLParamsStartHookPayload, OnGraphQLParamsStartHookResult},
    plugin_trait::{RouterPlugin, RouterPluginWithConfig, StartHookPayload},
};

#[derive(Deserialize)]
pub struct ForbidAnonymousOperationsPluginConfig {
    pub enabled: bool,
}
pub struct ForbidAnonymousOperationsPlugin {}

impl RouterPluginWithConfig for ForbidAnonymousOperationsPlugin {
    type Config = ForbidAnonymousOperationsPluginConfig;
    fn plugin_name() -> &'static str {
        "forbid_anonymous_operations"
    }
    fn from_config(config: Self::Config) -> Option<Self> {
        if config.enabled {
            Some(ForbidAnonymousOperationsPlugin {})
        } else {
            None
        }
    }
}

#[async_trait::async_trait]
impl RouterPlugin for ForbidAnonymousOperationsPlugin {
    async fn on_graphql_params<'exec>(
        &'exec self,
        payload: OnGraphQLParamsStartHookPayload<'exec>,
    ) -> OnGraphQLParamsStartHookResult<'exec> {
        let maybe_operation_name = &payload
            .graphql_params
            .as_ref()
            .and_then(|params| params.operation_name.as_ref());

        if maybe_operation_name.is_none()
            || maybe_operation_name
                .expect("is_none() has been checked before; qed")
                .is_empty()
        {
            // let's log the error
            tracing::error!("Operation is not allowed!");

            // Prepare an HTTP 400 response with a GraphQL error message
            let body = json!({
                "errors": [
                    {
                        "message": "Anonymous operations are not allowed",
                        "extensions": {
                            "code": "ANONYMOUS_OPERATION"
                        }
                    }
                ]
            });
            return payload.end_response(HttpResponse {
                body: sonic_rs::to_vec(&body).unwrap_or_default().into(),
                headers: http::HeaderMap::new(),
                status: StatusCode::BAD_REQUEST,
            });
        }
        // we're good to go!
        tracing::info!("operation is allowed!");
        payload.cont()
    }
}

#[cfg(test)]
mod tests {
    use crate::testkit::{init_router_from_config_inline, wait_for_readiness, SubgraphsServer};
    use hive_router::PluginRegistry;
    use http::StatusCode;
    use ntex::web::test;
    use serde_json::{json, Value};
    #[ntex::test]
    async fn should_forbid_anonymous_operations() {
        SubgraphsServer::start().await;
        let app = init_router_from_config_inline(
            r#"
            plugins:
                forbid_anonymous_operations:
                    enabled: true
        "#,
            Some(PluginRegistry::new().register::<super::ForbidAnonymousOperationsPlugin>()),
        )
        .await
        .expect("failed to start router");
        wait_for_readiness(&app.app).await;

        let resp = test::call_service(
            &app.app,
            test::TestRequest::post()
                .uri("/graphql")
                .set_payload(r#"{"query":"{ __schema { types { name } } }"}"#)
                .header("content-type", "application/json")
                .to_request(),
        )
        .await;

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let json_body: Value = serde_json::from_slice(&test::read_body(resp).await).unwrap();
        assert_eq!(
            json_body,
            json!({
                "errors": [
                    {
                        "message": "Anonymous operations are not allowed",
                        "extensions": {
                            "code": "ANONYMOUS_OPERATION"
                        }
                    }
                ]
            })
        );
    }
}
