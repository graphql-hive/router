#[cfg(test)]
mod tests {
    use std::time::Duration;

    use e2e::mockito;
    use e2e::testkit::{TestRouterBuilder, TestSubgraphsBuilder};
    use hive_router::ntex;
    use hive_router::tokio::time::sleep;
    use serde_json::json;

    #[ntex::test]
    async fn flushes_reports_on_shutdown() {
        let query = "query Test {me{name}}";
        let operation_name = "Test";

        let mut server = mockito::Server::new_async().await;
        let usage_mock = server
            .mock("POST", "/usage_report")
            .with_status(200)
            .expect(1)
            .match_body(mockito::Matcher::Json(json!([
                {
                    "query": query.to_string(),
                    "operation_name": operation_name.to_string(),
                }
            ])))
            .create_async()
            .await;

        {
            let subgraphs = TestSubgraphsBuilder::new().build().start().await;

            let router = TestRouterBuilder::new()
                .with_subgraphs(&subgraphs)
                .inline_config(
                    include_str!("../../../plugin_examples/usage_reporting/router.config.yaml")
                        .replace("0.0.0.0:9876", server.host_with_port().as_str()),
                )
                .register_plugin::<crate::plugin::UsageReportingPlugin>()
                .build()
                .start()
                .await;

            let res = router.send_graphql_request(query, None, None).await;
            assert!(res.status().is_success(), "Expected 200 OK");
        } // router and subgraphs drop here, triggering shutdown

        usage_mock.assert_async().await;
    }

    #[ntex::test]
    async fn flushes_reports_on_interval() {
        let query = "query Test {me{name}}";
        let operation_name = "Test";

        let mut server = mockito::Server::new_async().await;
        let usage_mock = server
            .mock("POST", "/usage_report")
            .with_status(200)
            .expect(1)
            .match_body(mockito::Matcher::Json(json!([
                {
                    "query": query.to_string(),
                    "operation_name": operation_name.to_string(),
                }
            ])))
            .create_async()
            .await;

        let subgraphs = TestSubgraphsBuilder::new().build().start().await;

        // Initialize with shorter interval so we can see if reports are flushed without waiting for shutdown
        let router = TestRouterBuilder::new()
            .with_subgraphs(&subgraphs)
            .inline_config(format!(
                r#"
                supergraph:
                    source: file
                    path: ../../e2e/supergraph.graphql
                plugins:
                    usage_reporting:
                        enabled: true
                        config:
                            endpoint: "http://{}/usage_report"
                            interval: "1s"
                "#,
                server.host_with_port()
            ))
            .register_plugin::<crate::plugin::UsageReportingPlugin>()
            .build()
            .start()
            .await;

        let res = router.send_graphql_request(query, None, None).await;
        assert!(res.status().is_success(), "Expected 200 OK");

        sleep(Duration::from_secs(2)).await; // Wait for the flush interval to pass

        usage_mock.assert_async().await;
    }
}
