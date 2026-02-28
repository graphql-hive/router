#[cfg(test)]
mod apollo_sandbox_tests {
    use e2e::testkit::{ClientResponseExt, TestRouterBuilder};
    use hive_router::ntex;

    #[ntex::test]
    async fn renders_apollo_sandbox_page() {
        let router = TestRouterBuilder::new()
            .file_config("../plugin_examples/apollo_sandbox/router.config.yaml")
            .register_plugin::<crate::plugin::ApolloSandboxPlugin>()
            .build()
            .start()
            .await;

        let res = router.serv().get("/apollo-sandbox").send().await.unwrap();

        let status = res.status();
        assert_eq!(status, 200);

        let body = res.string_body().await;
        assert!(body.contains("EmbeddedSandbox"));
    }
}
