#[cfg(test)]
mod override_subgraph_urls_e2e_tests {
    use crate::{
        some_header_map,
        testkit::{ClientResponseExt, TestRouterBuilder, TestSubgraphsBuilder},
    };

    #[ntex::test]
    /// Test that a static URL override for a subgraph is respected.
    /// Starts a subgraph server on port 4100, but the supergraph SDL points to 4200.
    /// The router config overrides the URL for the "accounts" subgraph to point to 4100.
    /// This way we can verify that the override is applied correctly.
    async fn should_override_subgraph_url_based_on_static_value() {
        let subgraphs = TestSubgraphsBuilder::new().build().start().await;
        let subgraphs_addr = subgraphs.addr();

        let router = TestRouterBuilder::new()
            .inline_config(format!(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                override_subgraph_urls:
                    accounts:
                        url: "http://{subgraphs_addr}/accounts"
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
        let subgraphs = TestSubgraphsBuilder::new().build().start().await;
        let subgraphs_addr = subgraphs.addr();

        let router = TestRouterBuilder::new()
            .inline_config(format!(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                override_subgraph_urls:
                    accounts:
                        url:
                            expression: |
                                if .request.headers."x-accounts-port" == "4100" {{
                                    "http://{subgraphs_addr}/accounts"
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
}
