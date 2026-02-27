#[cfg(test)]
mod tests {
    use e2e::testkit::{ClientResponseExt, TestRouterBuilder, TestSubgraphsBuilder};
    use hive_router::ntex;

    #[ntex::test]
    async fn accepts_non_standard_request_types() {
        let subgraphs = TestSubgraphsBuilder::new().build().start().await;

        let router = TestRouterBuilder::new()
            .with_subgraphs(&subgraphs)
            .file_config("../plugin_examples/non_standard_request/router.config.yaml")
            .register_plugin::<crate::plugin::NonStandardRequestPlugin>()
            .build()
            .start()
            .await;

        let res = router
            .serv()
            .post(router.graphql_path())
            .header("content-type", "text/plain;charset=UTF-8")
            .send_body(r#"{"query":"{ me { name } }"}"#)
            .await
            .unwrap();

        assert_eq!(
            res.string_body().await,
            r#"{"data":{"me":{"name":"Uri Goldshtein"}}}"#
        );

        subgraphs
            .get_requests_log("accounts")
            .expect("Should be able to get subgraph requests log");
    }
}
