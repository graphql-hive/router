#[cfg(test)]
mod tests {
    use e2e::testkit::{
        init_graphql_request, init_router_from_config_file_with_plugins, wait_for_readiness,
        SubgraphsServer,
    };
    use hive_router::ntex;
    use hive_router::ntex::web::test;
    use hive_router::PluginRegistry;
    #[ntex::test]
    async fn should_add_context_data_and_modify_subgraph_request() {
        let subgraphs = SubgraphsServer::start().await;

        let app = init_router_from_config_file_with_plugins(
            "../plugin_examples/context_data/router.config.yaml",
            PluginRegistry::new().register::<crate::plugin::ContextDataPlugin>(),
        )
        .await
        .expect("Router should initialize successfully");

        wait_for_readiness(&app.app).await;

        let resp = test::call_service(
            &app.app,
            init_graphql_request("{ users { id } }", None).to_request(),
        )
        .await;

        assert!(resp.status().is_success(), "Expected 200 OK");

        let request_logs = subgraphs
            .get_subgraph_requests_log("accounts")
            .await
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
