#[cfg(test)]
mod tests {
    use e2e::testkit::{
        init_router_from_config_file_with_plugins, wait_for_readiness, SubgraphsServer,
    };

    use hive_router::{
        http::StatusCode,
        ntex::{self, web::test},
        sonic_rs::{self, json},
        PluginRegistry,
    };
    #[ntex::test]
    async fn sends_not_found_error_if_query_missing() {
        let _ = SubgraphsServer::start().await;
        let app = init_router_from_config_file_with_plugins(
            "../plugin_examples/apq/router.config.yaml",
            PluginRegistry::new().register::<crate::plugin::APQPlugin>(),
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
        let body_json: sonic_rs::Value =
            sonic_rs::from_slice(&body).expect("Response body should be valid JSON");
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
        let subgraphs_server = SubgraphsServer::start().await;
        let app = init_router_from_config_file_with_plugins(
            "../plugin_examples/apq/router.config.yaml",
            PluginRegistry::new().register::<crate::plugin::APQPlugin>(),
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
        let resp_status = resp.status();
        let body = test::read_body(resp).await;
        let body_json: sonic_rs::Value =
            sonic_rs::from_slice(&body).expect("Response body should be valid JSON");
        println!("Response body: {}", body_json);
        assert_eq!(
            resp_status,
            StatusCode::OK,
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

        let subgraph_requests = subgraphs_server
            .get_subgraph_requests_log("accounts")
            .await
            .expect("expected requests sent to accounts subgraph");
        assert_eq!(
            subgraph_requests.len(),
            2,
            "expected 2 requests to accounts subgraph"
        );
    }
}
