#[cfg(test)]
mod tests {
    use e2e::testkit::{
        init_graphql_request, init_router_from_config_file_with_plugins, wait_for_readiness,
        SubgraphsServer,
    };

    use hive_router::{http, ntex, sonic_rs, BoxError, PluginRegistry};
    use ntex::web::test;
    #[ntex::test]
    async fn should_allow_only_allowed_client_ids() -> Result<(), BoxError> {
        let subgraphs_server = SubgraphsServer::start().await;

        let app = init_router_from_config_file_with_plugins(
            "../plugin_examples/async_auth/router.config.yaml",
            PluginRegistry::new().register::<crate::plugin::AllowClientIdFromFilePlugin>(),
        )
        .await?;
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
        let body_json: sonic_rs::Value = sonic_rs::from_slice(&body_bytes)?;
        assert_eq!(
            body_json,
            sonic_rs::json!({
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
        let body_json: sonic_rs::Value = sonic_rs::from_slice(&body_bytes)?;
        assert_eq!(
            body_json,
            sonic_rs::json!({
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

        let subgraph_requests = subgraphs_server
            .get_subgraph_requests_log("accounts")
            .await
            .expect("expected requests sent to accounts subgraph");
        assert_eq!(
            subgraph_requests.len(),
            1,
            "expected 1 request to accounts subgraph"
        );

        Ok(())
    }
}
