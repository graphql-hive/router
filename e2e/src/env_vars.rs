#[cfg(test)]
mod env_vars_e2e_tests {
    use sonic_rs::{from_slice, Value};

    use crate::testkit_v2::{EnvVarsGuard, TestRouterBuilder, TestSubgraphsBuilder};

    #[ntex::test]
    /// Test that a dynamic URL override for a subgraph based on an env var is respected.
    /// Starts a subgraph server on free port, and the supergraph SDL will be changed
    /// to point the allocated port.
    /// The router config overrides the URL for the "accounts" subgraph to point to 1000
    /// via the env var.
    /// This way we can verify that the override is applied correctly.
    /// Without the env var, the request goes to the allocated port (thanks to `.default`).
    async fn should_override_subgraph_url_based_on_env_var() {
        let subgraphs = TestSubgraphsBuilder::new().build().start().await;

        // Makes the expression to evaluate to port 4200 (value of .default)
        {
            let router = TestRouterBuilder::new()
                .with_subgraphs(&subgraphs)
                .file_config("configs/env_vars.router.yaml")
                .build()
                .start()
                .await;

            let res = router
                .send_graphql_request("{ users { id } }", None, None)
                .await;

            assert!(res.status().is_success(), "Expected 200 OK");

            let body = res.body().await.unwrap();
            let json_body: Value = from_slice(&body).unwrap();
            assert_eq!(json_body["data"]["users"][0]["id"], "1");

            let subgraph_requests = subgraphs
                .get_requests_log("accounts")
                .expect("expected requests sent to accounts subgraph");
            assert_eq!(
                subgraph_requests.len(),
                1,
                "expected 1 request to accounts subgraph"
            );

            drop(router); // Ensure router is dropped before subgraphs
        }

        // Makes the expression to evaluate to port 1000
        {
            let _env_guard = EnvVarsGuard::new()
                .set("ACCOUNTS_URL_OVERRIDE", "http://0.0.0.0:1000/accounts")
                .apply()
                .await;

            let router = TestRouterBuilder::new()
                .with_subgraphs(&subgraphs)
                .file_config("configs/env_vars.router.yaml")
                .build()
                .start()
                .await;

            let res = router
                .send_graphql_request("{ users { id } }", None, None)
                .await;

            assert!(res.status().is_success(), "Expected 200 OK");

            let body = res.body().await.unwrap();
            let json_body: Value = from_slice(&body).unwrap();

            insta::assert_snapshot!(sonic_rs::to_string_pretty(&json_body).unwrap(), @r#"
            {
              "data": {
                "users": null
              },
              "errors": [
                {
                  "message": "Failed to send request to subgraph \"http://0.0.0.0:1000/accounts\": client error (Connect)",
                  "extensions": {
                    "code": "SUBGRAPH_REQUEST_FAILURE",
                    "serviceName": "accounts"
                  }
                }
              ]
            }
            "#);

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

    #[ntex::test]
    /// Test that the `x-router-env` header value depends on the `ROUTER_ENV_HEADER` env var,
    /// with a fallback to "default".
    async fn should_insert_response_header_based_on_env_var() {
        let subgraphs = TestSubgraphsBuilder::new().build().start().await;

        // Makes the expression to evaluate to "default" (default value provided)
        {
            let router = TestRouterBuilder::new()
                .with_subgraphs(&subgraphs)
                .file_config("configs/env_vars.router.yaml")
                .build()
                .start()
                .await;

            let res = router
                .send_graphql_request("{ users { id } }", None, None)
                .await;

            assert!(res.status().is_success(), "Expected 200 OK");
            assert_eq!(
                res.headers()
                    .get("x-router-env")
                    .map(|v| v.to_str().unwrap()),
                Some("default")
            );

            drop(router); // Ensure router is dropped before subgraphs
        }

        // Makes the expression to evaluate to ROUTER_ENV_HEADER value
        {
            let _env_guard = EnvVarsGuard::new()
                .set("ROUTER_ENV_HEADER", "e2e")
                .apply()
                .await;

            let router = TestRouterBuilder::new()
                .with_subgraphs(&subgraphs)
                .file_config("configs/env_vars.router.yaml")
                .build()
                .start()
                .await;

            let res = router
                .send_graphql_request("{ users { id } }", None, None)
                .await;

            assert!(res.status().is_success(), "Expected 200 OK");
            assert_eq!(
                res.headers()
                    .get("x-router-env")
                    .map(|v| v.to_str().unwrap()),
                Some("e2e")
            );
        }
    }
}
