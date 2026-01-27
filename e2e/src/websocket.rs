#[cfg(test)]
mod websocket_e2e_tests {
    use crate::testkit_v2::TestRouterBuilder;
    use hive_router_plan_executor::executors::websocket_client::GraphQLTransportWSClient;

    #[ntex::test]
    async fn query_over_websocket() {
        let router = TestRouterBuilder::new()
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

        let wsconn = router.ws().await;

        let client = GraphQLTransportWSClient::new(wsconn);

        // TODO: implement sending and receiving messages over the websocket client
    }
}
