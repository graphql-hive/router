#[cfg(test)]
mod tests {
    use e2e::testkit::{
        init_router_from_config_file_with_plugins, wait_for_readiness, SubgraphsServer,
    };
    use hive_router::{
        http::Method,
        ntex::{self, http::test::TestRequest},
        PluginRegistry,
    };

    #[ntex::test]
    async fn accepts_non_standard_request_types() {
        let subgraphs = SubgraphsServer::start().await;
        let app = init_router_from_config_file_with_plugins(
            "../plugin_examples/non_standard_request/router.config.yaml",
            PluginRegistry::new().register::<crate::plugin::NonStandardRequestPlugin>(),
        )
        .await
        .expect("Router should initialize successfully");
        wait_for_readiness(&app.app).await;

        let req = TestRequest::with_uri("/graphql")
            .method(Method::POST)
            .set_payload(r#"{"query":"{ me { name } }"}"#)
            .header("content-type", "text/plain;charset=UTF-8")
            .finish();

        let resp = ntex::web::test::call_service(&app.app, req).await;
        let body = ntex::web::test::read_body(resp).await;
        assert_eq!(body, r#"{"data":{"me":{"name":"Uri Goldshtein"}}}"#);
        subgraphs
            .get_subgraph_requests_log("accounts")
            .await
            .expect("Should be able to get subgraph requests log");
    }
}
