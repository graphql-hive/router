#[cfg(test)]
mod tests {
    use e2e::testkit::{TestRouterBuilder, TestSubgraphsBuilder};
    use hive_router::ntex;

    #[ntex::test]
    async fn should_add_context_data_and_modify_subgraph_request() {
        let subgraphs = TestSubgraphsBuilder::new().build().start().await;

        let router = TestRouterBuilder::new()
            .with_subgraphs(&subgraphs)
            .file_config("../plugin_examples/context_data/router.config.yaml")
            .register_plugin::<crate::plugin::ContextDataPlugin>()
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request("{ users { id } }", None, None)
            .await;

        assert!(res.status().is_success(), "Expected 200 OK");

        let request_logs = subgraphs
            .get_requests_log("accounts")
            .expect("expected requests sent to accounts subgraph");
        assert_eq!(
            request_logs.len(),
            1,
            "expected 1 request to accounts subgraph"
        );
        let hello_header_value = request_logs[0]
            .headers
            .get("x-hello")
            .expect("expected x-hello header to be present in subgraph request")
            .to_str()
            .expect("header value should be valid string");
        assert_eq!(hello_header_value, "Hello world");
    }
}
