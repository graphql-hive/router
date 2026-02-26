#[cfg(test)]
mod tests {
    use e2e::{
        some_header_map,
        testkit::{ClientResponseExt, TestRouterBuilder, TestSubgraphsBuilder},
    };

    use hive_router::{http, ntex, sonic_rs};

    #[ntex::test]
    async fn should_allow_only_allowed_client_ids() {
        let subgraphs = TestSubgraphsBuilder::new().build().start().await;

        let router = TestRouterBuilder::new()
            .with_subgraphs(&subgraphs)
            .file_config("../plugin_examples/async_auth/router.config.yaml")
            .register_plugin::<crate::plugin::AllowClientIdFromFilePlugin>()
            .build()
            .start()
            .await;

        // Test with an allowed client id
        let res = router
            .send_graphql_request(
                "{ users { id } }",
                None,
                some_header_map! {
                    "x-client-id" => "urql"
                },
            )
            .await;
        assert!(
            res.status().is_success(),
            "Expected 200 OK for allowed client id"
        );

        // Test with a disallowed client id
        let res = router
            .send_graphql_request(
                "{ users { id } }",
                None,
                some_header_map! {
                    "x-client-id" => "forbidden-client"
                },
            )
            .await;
        assert_eq!(
            res.status(),
            http::StatusCode::FORBIDDEN,
            "Expected 403 FORBIDDEN for disallowed client id"
        );
        assert_eq!(
            res.json_body().await,
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
        let res = router
            .send_graphql_request("{ users { id } }", None, None)
            .await;
        assert_eq!(
            res.status(),
            http::StatusCode::UNAUTHORIZED,
            "Expected 401 UNAUTHORIZED for missing client id"
        );
        assert_eq!(
            res.json_body().await,
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

        let subgraph_requests = subgraphs
            .get_requests_log("accounts")
            .expect("expected requests sent to accounts subgraph");
        assert_eq!(
            subgraph_requests.len(),
            1,
            "expected 1 request to accounts subgraph"
        );
    }
}
