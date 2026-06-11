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

    #[ntex::test]
    async fn should_propagate_response_headers_on_failures() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let mut accounts_server = mockito::Server::new_async().await;
        let host = accounts_server.host_with_port();

        let router = TestRouter::builder()
            .inline_config(format!(
                r#"
                  supergraph:
                    source: file
                    path: supergraph.graphql
                  headers:
                    all:
                      response:
                        - propagate:
                            named: x-subgraph
                            algorithm: last
                  override_subgraph_urls:
                      subgraphs:
                          accounts:
                              url: "http://{host}/accounts"
                  "#
            ))
            .with_subgraphs(&subgraphs)
            .build()
            .start()
            .await;

        let accounts_response_mock = accounts_server
            .mock("POST", "/accounts")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_header("x-subgraph", "accounts")
            .expect(1)
            .create();

        let res = router
            .send_graphql_request("{ users { id } }", None, None)
            .await;

        assert!(res.status().is_success(), "Expected 200 OK");

        accounts_response_mock.assert();

        assert_eq!(
            res.headers()
                .get("x-subgraph")
                .and_then(|v| v.to_str().ok()),
            Some("accounts"),
            "expected x-subgraph header to be propagated"
        );
    }
}
