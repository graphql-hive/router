#[cfg(test)]
mod env_vars_e2e_tests {
    use ntex::web::test;
    use sonic_rs::{from_slice, Value};

    use crate::testkit::{
        init_graphql_request, init_router_from_config_file, wait_for_readiness, EnvVarGuard,
        SubgraphsServer,
    };

    #[ntex::test]
    /// Test that a dynamic URL override for a subgraph based on an env var is respected.
    /// Starts a subgraph server on port 4200, and the supergraph SDL points to 4200.
    /// The router config overrides the URL for the "accounts" subgraph to point to 4100
    /// via the env var.
    /// This way we can verify that the override is applied correctly.
    /// Without the env var, the request goes to 4200 (thanks to `.default`).
    async fn should_override_subgraph_url_based_on_env_var() {
        let subgraphs_server = SubgraphsServer::start_with_port(4200).await;

        // Makes the expression to evaluate to port 4200 (value of .default)
        {
            let app = init_router_from_config_file("configs/env_vars.router.yaml")
                .await
                .unwrap();
            wait_for_readiness(&app.app).await;

            let req = init_graphql_request("{ users { id } }", None);
            let resp = test::call_service(&app.app, req.to_request()).await;

            assert!(resp.status().is_success(), "Expected 200 OK");
            let body = test::read_body(resp).await;
            let json_body: Value = from_slice(&body).unwrap();
            assert_eq!(json_body["data"]["users"][0]["id"], "1");

            let subgraph_requests = subgraphs_server
                .get_subgraph_requests_log("accounts")
                .await
                .expect("expected requests sent to accounts subgraph");
            assert_eq!(
                subgraph_requests.len(),
                1,
                "expected 1 request to accounts subgraph"
            );

            drop(app); // Ensure app is dropped before subgraphs_server
        }

        // Makes the expression to evaluate to port 4100
        {
            let _env_guard =
                EnvVarGuard::new("ACCOUNTS_URL_OVERRIDE", "http://0.0.0.0:4100/accounts");

            let app = init_router_from_config_file("configs/env_vars.router.yaml")
                .await
                .unwrap();
            wait_for_readiness(&app.app).await;

            let req = init_graphql_request("{ users { id } }", None);
            let resp = test::call_service(&app.app, req.to_request()).await;

            assert!(resp.status().is_success(), "Expected 200 OK");
            let body = test::read_body(resp).await;
            let json_body: Value = from_slice(&body).unwrap();

            insta::assert_snapshot!(sonic_rs::to_string_pretty(&json_body).unwrap(), @r###"
            {
                "errors": [
                    {
                        "message": "Failed to send request to subgraph \"http://0.0.0.0:4100/accounts\": client error (Connect)",
                        "extensions": {
                            "code": "SUBGRAPH_REQUEST_FAILURE",
                            "serviceName": "accounts"
                        }
                    }
                ]
            }"###);

            let subgraph_requests = subgraphs_server
                .get_subgraph_requests_log("accounts")
                .await
                .expect("expected requests sent to accounts subgraph");
            assert_eq!(
                subgraph_requests.len(),
                1,
                "expected 1 request to accounts subgraph"
            );
        }
    }

    #[ntex::test]
    /// Test that the `x-router-env` header value depends on the `ROUTER_ENV_HEADER` env var,
    /// with a fallback to "default".
    async fn should_insert_response_header_based_on_env_var() {
        let _subgraphs_server = SubgraphsServer::start().await;

        // Makes the expression to evaluate to "default" (default value provided)
        {
            let app = init_router_from_config_file("configs/env_vars.router.yaml")
                .await
                .unwrap();
            wait_for_readiness(&app.app).await;

            let req = init_graphql_request("{ users { id } }", None);
            let resp = test::call_service(&app.app, req.to_request()).await;

            assert!(resp.status().is_success(), "Expected 200 OK");
            assert_eq!(
                resp.headers()
                    .get("x-router-env")
                    .map(|v| v.to_str().unwrap()),
                Some("default")
            );

            drop(app); // Ensure app is dropped before subgraphs_server
        }

        // Makes the expression to evaluate to ROUTER_ENV_HEADER value
        {
            let _env_guard = EnvVarGuard::new("ROUTER_ENV_HEADER", "e2e");

            let app = init_router_from_config_file("configs/env_vars.router.yaml")
                .await
                .unwrap();
            wait_for_readiness(&app.app).await;

            let req = init_graphql_request("{ users { id } }", None);
            let resp = test::call_service(&app.app, req.to_request()).await;

            assert!(resp.status().is_success(), "Expected 200 OK");
            assert_eq!(
                resp.headers()
                    .get("x-router-env")
                    .map(|v| v.to_str().unwrap()),
                Some("e2e")
            );
        }
    }
}
