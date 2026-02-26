#[cfg(test)]
mod tests {
    use std::time::Duration;

    use e2e::mockito::{self, ServerOpts};
    use e2e::testkit::{
        init_graphql_request, init_router_from_config_file_with_plugins,
        init_router_from_config_inline_with_plugins, wait_for_readiness, SubgraphsServer,
    };
    use hive_router::ntex::web::test;
    use hive_router::tokio::time::sleep;
    use hive_router::{ntex, BoxError, PluginRegistry};
    use serde_json::json;
    #[ntex::test]
    async fn flushes_reports_on_shutdown() -> Result<(), BoxError> {
        let query = "query Test {me{name}}";
        let operation_name = "Test";
        let mut server = mockito::Server::new_with_opts_async(ServerOpts {
            port: 9876,
            ..Default::default()
        })
        .await;
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
            let _subgraphs_server = SubgraphsServer::start().await;

            let test_app = init_router_from_config_file_with_plugins(
                "../plugin_examples/usage_reporting/router.config.yaml",
                PluginRegistry::new().register::<crate::plugin::UsageReportingPlugin>(),
            )
            .await?;

            wait_for_readiness(&test_app.app).await;

            let resp = test::call_service(
                &test_app.app,
                init_graphql_request(query, None).to_request(),
            )
            .await;
            assert!(resp.status().is_success(), "Expected 200 OK");

            test_app.shutdown().await;
        }

        usage_mock.assert_async().await;

        Ok(())
    }
    #[ntex::test]
    async fn flushes_reports_on_interval() -> Result<(), BoxError> {
        let query = "query Test {me{name}}";
        let operation_name = "Test";
        let mut server = mockito::Server::new_with_opts_async(ServerOpts {
            port: 9876,
            ..Default::default()
        })
        .await;
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
        let _subgraphs_server = SubgraphsServer::start().await;

        // Initialize with shorter interval so we can see if reports are flushed without waiting for shutdown
        let test_app = init_router_from_config_inline_with_plugins(
            r#"
                supergraph:
                    source: file
                    path: ../../e2e/supergraph.graphql
                plugins:
                    usage_reporting:
                        enabled: true
                        config:
                            endpoint: "http://localhost:9876/usage_report"
                            interval: "1s"
                "#,
            PluginRegistry::new().register::<crate::plugin::UsageReportingPlugin>(),
        )
        .await?;

        wait_for_readiness(&test_app.app).await;

        let resp = test::call_service(
            &test_app.app,
            init_graphql_request(query, None).to_request(),
        )
        .await;
        assert!(resp.status().is_success(), "Expected 200 OK");

        sleep(Duration::from_secs(2)).await; // Wait for the flush interval to pass

        usage_mock.assert_async().await;

        test_app.shutdown().await;

        Ok(())
    }
}
