#[cfg(test)]
mod tests {
    use e2e::mockito;
    use e2e::testkit::{ClientResponseExt, TestRouterBuilder};
    use hive_router::{http::StatusCode, ntex};

    #[ntex::test]
    async fn should_map_subgraph_errors() {
        let mut subgraphs = mockito::Server::new_async().await;

        let router = TestRouterBuilder::new()
            .with_subgraphs(subgraphs.socket_address())
            .file_config("../plugin_examples/error_mapping/router.config.yaml")
            .register_plugin::<crate::plugin::ErrorMappingPlugin>()
            .build()
            .start()
            .await;

        let mock = subgraphs
            .mock("POST", "/accounts")
            .with_header("content-type", "application/json")
            .with_body(r#"{"errors":[{"message":"My Error"}]}"#)
            .create_async()
            .await;

        let res = router
            .send_graphql_request("{ users { id } }", None, None)
            .await;

        assert_eq!(res.status(), StatusCode::from_u16(502).unwrap());

        assert!(
            res.str_body().await.contains(r#""code":"BadGateway""#),
            "Expected error code to be BadGateway"
        );

        mock.assert_async().await;
    }

    #[ntex::test]
    async fn should_map_router_errors() {
        let router = TestRouterBuilder::new()
            .file_config("../plugin_examples/error_mapping/router.config.yaml")
            .register_plugin::<crate::plugin::ErrorMappingPlugin>()
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request("{ users { id", None, None)
            .await;

        assert!(res.status().is_client_error(), "Expected 4xx status code");

        assert!(
            res.str_body().await.contains(r#""code":"InvalidInput""#),
            "Expected error code to be InvalidInput"
        );
    }
}
