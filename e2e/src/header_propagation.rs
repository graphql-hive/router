#[cfg(test)]

mod header_propagation_e2e_tests {
    use crate::testkit::{some_header_map, TestRouter, TestSubgraphs};

    #[ntex::test]
    async fn should_propagate_headers_to_subgraphs() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .file_config("configs/header_propagation.router.yaml")
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(
                "{ users { id } }",
                None,
                some_header_map! {
                    http::header::HeaderName::from_static("x-context") => "my-context-value"
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

        let last_request = &subgraph_requests[0];
        let context_header = last_request
            .headers
            .get("x-context")
            .expect("expected x-context header to be present");
        assert_eq!(
            context_header, "my-context-value",
            "expected x-context header to be propagated to subgraph"
        );
    }

    // Regression test for https://github.com/graphql-hive/router/issues/997
    //
    // When a header configured to be propagated to subgraphs is sent by the
    // client with an empty value, the router used to panic in the ntex-http
    // crate while constructing the outgoing subgraph request.
    //
    // The router must instead handle the empty value gracefully (either
    // propagate it or drop it) and return a successful response.
    #[ntex::test]
    async fn should_not_panic_when_propagated_header_has_empty_value() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .file_config("configs/header_propagation.router.yaml")
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(
                "{ users { id } }",
                None,
                some_header_map! {
                    http::header::HeaderName::from_static("x-context") => ""
                },
            )
            .await;

        assert!(
            res.status().is_success(),
            "Expected 200 OK, got {} (router likely panicked while propagating empty header value)",
            res.status()
        );

        let subgraph_requests = subgraphs
            .get_requests_log("accounts")
            .expect("expected requests sent to accounts subgraph");
        assert_eq!(
            subgraph_requests.len(),
            1,
            "expected 1 request to accounts subgraph"
        );

        let last_request = &subgraph_requests[0];
        if let Some(context_header) = last_request.headers.get("x-context") {
            assert_eq!(
                context_header, "",
                "expected x-context header to be propagated as empty"
            );
        }
    }
}
