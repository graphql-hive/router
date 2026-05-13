#[cfg(test)]
mod override_subgraph_urls_e2e_tests {
    use crate::{
        some_header_map,
        testkit::{ClientResponseExt, TestRouter, TestSubgraphs},
    };
    use sonic_rs::json;

    #[ntex::test]
    /// Test that a static URL override for a subgraph is respected.
    /// Starts a subgraph server on port 4100, but the supergraph SDL points to 4200.
    /// The router config overrides the URL for the "accounts" subgraph to point to 4100.
    /// This way we can verify that the override is applied correctly.
    async fn should_override_subgraph_url_based_on_static_value() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let subgraphs_url = subgraphs.url();

        let router = TestRouter::builder()
            .inline_config(format!(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                override_subgraph_urls:
                    subgraphs:
                        accounts:
                            url: "{subgraphs_url}/accounts"
                "#,
            ))
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request("{ users { id } }", None, None)
            .await;

        assert!(res.status().is_success(), "Expected 200 OK");

        let subgraph_requests = subgraphs
            .get_requests_log("accounts")
            .expect("expected requests sent to accounts subgraph");
        assert_eq!(
            subgraph_requests.len(),
            1,
            "expected 1 request to accounts subgraph"
        );
    }

    #[ntex::test]
    /// Test that a dynamic URL override for a subgraph based on a header value is respected.
    /// Starts a subgraph server on port 4100, but the supergraph SDL points to 4200.
    /// The router config overrides the URL for the "accounts" subgraph to point to 4100
    /// when a specific header is present.
    /// This way we can verify that the override is applied correctly.
    /// Without the header, the request goes to 4200 and fail (thanks to `.default`).
    async fn should_override_subgraph_url_based_on_header_value() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let subgraphs_url = subgraphs.url();

        let router = TestRouter::builder()
            .inline_config(format!(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                override_subgraph_urls:
                    subgraphs:
                        accounts:
                            url:
                                expression: |
                                    if .request.headers."x-accounts-port" == "4100" {{
                                        "{subgraphs_url}/accounts"
                                    }} else {{
                                        .default
                                    }}
                "#,
            ))
            .build()
            .start()
            .await;

        // Makes the expression to evaluate to port 4100
        let res = router
            .send_graphql_request(
                "{ users { id } }",
                None,
                some_header_map! {
                    "x-accounts-port" => "4100"
                },
            )
            .await;

        assert!(res.status().is_success(), "Expected 200 OK");

        let subgraph_requests = subgraphs
            .get_requests_log("accounts")
            .expect("expected requests sent to accounts subgraph");
        assert_eq!(
            subgraph_requests.len(),
            1,
            "expected 1 request to accounts subgraph"
        );

        // Makes the expression to evaluate to port 4200 (value of .default)
        // which is not running, so the request fails
        let res = router
            .send_graphql_request("{ users { id } }", None, None)
            .await;

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
        {
          "data": {
            "users": null
          },
          "errors": [
            {
              "message": "Failed to send request to subgraph: client error (Connect)",
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
            1, // still 1, no new request should be made
            "expected 1 request to accounts subgraph"
        );
    }

    #[ntex::test]
    /// Test that `override_subgraph_urls.all` is applied to every subgraph
    /// that doesn't have its own per-subgraph override.
    /// The expression branches on `subgraph.name`, sending each subgraph to
    /// the matching test subgraph URL.
    async fn should_apply_all_expression_to_every_subgraph() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let subgraphs_url = subgraphs.url();

        let router = TestRouter::builder()
            .inline_config(format!(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                override_subgraph_urls:
                    all:
                        expression: |
                            if .subgraph.name == "accounts" {{
                                "{subgraphs_url}/accounts"
                            }} else if .subgraph.name == "reviews" {{
                                "{subgraphs_url}/reviews"
                            }} else if .subgraph.name == "products" {{
                                "{subgraphs_url}/products"
                            }} else if .subgraph.name == "inventory" {{
                                "{subgraphs_url}/inventory"
                            }} else {{
                                .default
                            }}
                "#,
            ))
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request("{ users { id name } }", None, None)
            .await;

        assert!(res.status().is_success(), "Expected 200 OK");

        let accounts_requests = subgraphs
            .get_requests_log("accounts")
            .expect("expected requests sent to accounts subgraph");
        assert_eq!(
            accounts_requests.len(),
            1,
            "expected 1 request to accounts subgraph"
        );
    }

    #[ntex::test]
    /// Test that per-subgraph overrides take precedence over `all`. The
    /// `accounts` subgraph keeps its per-subgraph static URL while every
    /// other subgraph falls back to the `all` expression (here, `.default`,
    /// which points to a non-running port and would fail).
    async fn should_prefer_per_subgraph_override_over_all() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let subgraphs_url = subgraphs.url();

        let router = TestRouter::builder()
            .inline_config(format!(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                override_subgraph_urls:
                    subgraphs:
                        accounts:
                            url: "{subgraphs_url}/accounts"
                    all:
                        expression: |
                            "http://0.0.0.0:1/should-not-be-used"
                "#,
            ))
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request("{ users { id } }", None, None)
            .await;

        assert!(res.status().is_success(), "Expected 200 OK");

        let accounts_requests = subgraphs
            .get_requests_log("accounts")
            .expect("expected requests sent to accounts subgraph");
        assert_eq!(
            accounts_requests.len(),
            1,
            "the per-subgraph override should win over `all`"
        );
    }

    #[ntex::test]
    /// Test that `all` is reactive to request headers and can therefore
    /// produce different per-subgraph URLs without enumerating each
    /// subgraph individually.
    async fn should_apply_all_expression_dynamically() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let subgraphs_url = subgraphs.url();

        let router = TestRouter::builder()
            .inline_config(format!(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                override_subgraph_urls:
                    all:
                        expression: |
                            if .request.headers."x-route-to" == "accounts" {{
                                "{subgraphs_url}/accounts"
                            }} else {{
                                .default
                            }}
                "#,
            ))
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(
                "{ users { id } }",
                None,
                some_header_map! { "x-route-to" => "accounts" },
            )
            .await;

        assert!(res.status().is_success(), "Expected 200 OK");

        let accounts_requests = subgraphs
            .get_requests_log("accounts")
            .expect("expected requests sent to accounts subgraph");
        assert_eq!(
            accounts_requests.len(),
            1,
            "expected 1 request to accounts subgraph"
        );
    }

    #[ntex::test]
    /// Test that path parameters captured from a wildcard `graphql_endpoint`
    /// are exposed to override expressions through `.request.url_matches`.
    /// The router is configured with `/{tenant}/graphql` so requests like
    /// `/acme/graphql` capture `tenant=acme` and the expression rewrites
    /// the subgraph URL accordingly.
    async fn should_expose_graphql_endpoint_path_params_to_override_expression() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let subgraphs_url = subgraphs.url();

        let router = TestRouter::builder()
            .inline_config(format!(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                http:
                    graphql_endpoint: /{{tenant}}/graphql
                override_subgraph_urls:
                    all:
                        expression: |
                            tenant = string!(.request.url_matches.tenant)
                            if tenant == "acme" {{
                                "{subgraphs_url}/" + string!(.subgraph.name)
                            }} else {{
                                .default
                            }}
                "#,
            ))
            .build()
            .start()
            .await;

        let res = router
            .send_post_request(
                "/acme/graphql",
                json!({
                    "query": "{ users { id } }",
                }),
                None,
            )
            .await;

        assert!(res.status().is_success(), "Expected 200 OK");

        let accounts_requests = subgraphs
            .get_requests_log("accounts")
            .expect("expected requests sent to accounts subgraph");
        assert_eq!(
            accounts_requests.len(),
            1,
            "expected 1 request to accounts subgraph after URL rewrite"
        );
    }
}
