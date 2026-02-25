#[cfg(test)]
mod tests {
    use e2e::testkit::{
        init_router_from_config_file_with_plugins, wait_for_readiness, SubgraphsServer,
    };
    use hive_router::http::StatusCode;
    use hive_router::ntex::web::test;
    use hive_router::sonic_rs::{json, Value};
    use hive_router::{ntex, sonic_rs, PluginRegistry};
    #[ntex::test]
    async fn should_forbid_anonymous_operations() {
        SubgraphsServer::start().await;

        let app = init_router_from_config_file_with_plugins(
            "../plugin_examples/forbid_anonymous_operations/router.config.yaml",
            PluginRegistry::new().register::<crate::plugin::ForbidAnonymousOperationsPlugin>(),
        )
        .await
        .expect("Router should initialize successfully");
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
        let json_body: Value = sonic_rs::from_slice(&test::read_body(resp).await).unwrap();
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
