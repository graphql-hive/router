#[cfg(test)]
mod override_subgraph_urls_e2e_tests {
    use ntex::web::test;
    use sonic_rs::{from_slice, Value};

    use crate::testkit::{
        init_graphql_request, init_router_from_config_file, wait_for_readiness, SubgraphsServer,
    };

    #[ntex::test]
    /// Test that a static URL override for a subgraph is respected.
    /// Starts a subgraph server on port 4100, but the supergraph SDL points to 4200.
    /// The router config overrides the URL for the "accounts" subgraph to point to 4100.
    /// This way we can verify that the override is applied correctly.
    async fn should_override_subgraph_url_based_on_static_value() {
        let subgraphs_server = SubgraphsServer::start_with_port(4100).await;
        let app = init_router_from_config_file(
            "configs/override_subgraph_urls/override_static.router.yaml", None,
        )
        .await
        .unwrap();
        wait_for_readiness(&app.app).await;

        let req = init_graphql_request("{ users { id } }", None);
        let resp = test::call_service(&app.app, req.to_request()).await;

        assert!(resp.status().is_success(), "Expected 200 OK");

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

    #[ntex::test]
    /// Test that a dynamic URL override for a subgraph based on a header value is respected.
    /// Starts a subgraph server on port 4100, but the supergraph SDL points to 4200.
    /// The router config overrides the URL for the "accounts" subgraph to point to 4100
    /// when a specific header is present.
    /// This way we can verify that the override is applied correctly.
    /// Without the header, the request goes to 4200 and fail (thanks to `.original_url`).
    async fn should_override_subgraph_url_based_on_header_value() {
        let subgraphs_server = SubgraphsServer::start_with_port(4100).await;
        let app = init_router_from_config_file(
            "configs/override_subgraph_urls/override_dynamic_header.router.yaml",
            None,
        )
        .await
        .unwrap();
        wait_for_readiness(&app.app).await;
        // Makes the expression to evaluate to port 4100
        let req = init_graphql_request("{ users { id } }", None).header("x-accounts-port", "4100");
        let resp = test::call_service(&app.app, req.to_request()).await;

        assert!(resp.status().is_success(), "Expected 200 OK");

        let subgraph_requests = subgraphs_server
            .get_subgraph_requests_log("accounts")
            .await
            .expect("expected requests sent to accounts subgraph");
        assert_eq!(
            subgraph_requests.len(),
            1,
            "expected 1 request to accounts subgraph"
        );

        // Makes the expression to evaluate to port 4200 (value of .original_url)
        // which is not running, so the request fails
        let req = init_graphql_request("{ users { id } }", None);
        let resp = test::call_service(&app.app, req.to_request()).await;

        assert!(resp.status().is_success(), "Expected 200 OK");
        let body = test::read_body(resp).await;
        let json_body: Value = from_slice(&body).unwrap();

        assert_eq!(
            json_body["errors"][0]["message"],
            "Failed to execute request to subgraph"
        );
        assert_eq!(
            json_body["errors"][0]["extensions"]["code"],
            "SUBGRAPH_REQUEST_FAILURE"
        );

        let subgraph_requests = subgraphs_server
            .get_subgraph_requests_log("accounts")
            .await
            .expect("expected requests sent to accounts subgraph");
        assert_eq!(
            subgraph_requests.len(),
            1, // still 1, no new request should be made
            "expected 1 request to accounts subgraph"
        );
    }
}
