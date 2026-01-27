#[cfg(test)]
mod websocket_e2e_tests {
    use insta::assert_snapshot;

    use reqwest::StatusCode;
    use sonic_rs::from_slice;

    use crate::testkit_v2::TestServerBuilder;

    #[ntex::test]
    async fn query_over_websocket() {
        let router = TestServerBuilder::new()
            .with_subgraphs()
            .inline_config(&format!(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                "#
            ))
            .build()
            .start()
            .await
            .expect("Failed to start test router");

        let mut res = router
            .send_graphql_request("{ topProducts { name }}", None)
            .await;

        assert_eq!(res.status(), StatusCode::OK, "Expected 200 OK");

        let body_bytes = res.body().await.expect("Failed to read response body");
        let body: sonic_rs::Value =
            from_slice(&body_bytes).expect("Response body is not valid JSON");

        assert_snapshot!(body, @r#"{"data":{"topProducts":[{"name":"Table"},{"name":"Couch"},{"name":"Glass"},{"name":"Chair"},{"name":"TV"}]}}"#);
    }
}
