#[cfg(test)]
mod router_timeout_e2e_tests {
    use std::{thread::sleep, time::Duration};

    use crate::testkit_v2::TestRouterBuilder;

    #[ntex::test]
    async fn should_timeout_request_when_exceeding_router_timeout() {
        let mut accounts_server = mockito::Server::new_async().await;
        let host = accounts_server.host_with_port();

        let mock = accounts_server
            .mock("POST", "/accounts")
            .with_status(200)
            .with_chunked_body(|writer| {
                sleep(Duration::from_secs(10));
                writer.write_all(b"{\"data\":{\"users\":[{\"id\":\"1\"}]}}")
            })
            .create_async()
            .await;

        // This test just ensures that when a request takes longer than the configured router timeout, it gets timed out and doesn't cause any deadlocks or other issues in the router.
        // The actual timeout behavior is tested in unit tests for the timeout middleware, so here we just want to ensure that it works correctly in an end-to-end scenario with the full router setup.
        let router = TestRouterBuilder::new()
            .inline_config(&format!(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                traffic_shaping:
                    router:
                        request_timeout: 2s
                override_subgraph_urls:
                    accounts:
                        url: "http://{host}/accounts"
                "#
            ))
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request("{ users { id } }", None, None)
            .await;

        assert_eq!(
            res.status(),
            ntex::http::StatusCode::GATEWAY_TIMEOUT,
            "Expected 504 Gateway Timeout"
        );

        let body = res.body().await.unwrap();
        let body_str = std::str::from_utf8(&body).unwrap();
        insta::assert_snapshot!(body_str, @r#"{"errors":[{"message":"Request timed out","extensions":{"code":"GATEWAY_TIMEOUT"}}]}"#);

        // Ensure that the accounts server received the request, which means that the router did forward the request to the subgraph, but then timed it out correctly.
        mock.assert_async().await;
    }
}
