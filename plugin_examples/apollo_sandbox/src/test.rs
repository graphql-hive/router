#[cfg(test)]
mod apollo_sandbox_tests {
    use e2e::testkit::init_router_from_config_file_with_plugins;
    use hive_router::{ntex, PluginRegistry};

    #[ntex::test]
    async fn renders_apollo_sandbox_page() {
        use ntex::web::test;

        let app = init_router_from_config_file_with_plugins(
            "../plugin_examples/apollo_sandbox/router.config.yaml",
            PluginRegistry::new().register::<crate::plugin::ApolloSandboxPlugin>(),
        )
        .await
        .expect("failed to start router");

        let req = test::TestRequest::get().uri("/apollo-sandbox").to_request();
        let response = app.call(req).await.expect("failed to call /apollo-sandbox");
        let status = response.status();

        let body_bytes = test::read_body(response).await;
        let body_str = std::str::from_utf8(&body_bytes).expect("response body is not valid UTF-8");

        assert_eq!(status, 200);
        assert!(body_str.contains("EmbeddedSandbox"));
    }
}
