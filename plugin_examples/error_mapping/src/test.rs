#[cfg(test)]
mod tests {
    use e2e::mockito::{self, ServerOpts};
    use e2e::testkit::{
        init_graphql_request, init_router_from_config_file_with_plugins, wait_for_readiness,
    };
    use hive_router::http::StatusCode;
    use hive_router::ntex;
    use hive_router::ntex::web::test;
    use hive_router::PluginRegistry;
    #[ntex::test]
    async fn should_map_subgraph_errors() {
        let app = init_router_from_config_file_with_plugins(
            "../plugin_examples/error_mapping/router.config.yaml",
            PluginRegistry::new().register::<crate::plugin::ErrorMappingPlugin>(),
        )
        .await
        .expect("Router should initialize successfully");

        let mut subgraphs_server = mockito::Server::new_with_opts_async(ServerOpts {
            port: 4200,
            ..Default::default()
        })
        .await;

        let mock = subgraphs_server
            .mock("POST", "/accounts")
            .with_header("content-type", "application/json")
            .with_body(r#"{"errors":[{"message":"My Error"}]}"#)
            .create_async()
            .await;

        wait_for_readiness(&app.app).await;

        let resp = test::call_service(
            &app.app,
            init_graphql_request("{ users { id } }", None).to_request(),
        )
        .await;

        assert_eq!(resp.status(), StatusCode::from_u16(502).unwrap());
        let body = test::read_body(resp).await;
        let body_str = std::str::from_utf8(&body).expect("Response body should be valid UTF-8");
        assert!(
            body_str.contains(r#""code":"BadGateway""#),
            "Expected error code to be BadGateway"
        );
        mock.assert_async().await;
    }
    #[ntex::test]
    async fn should_map_router_errors() {
        let app = init_router_from_config_file_with_plugins(
            "../plugin_examples/error_mapping/router.config.yaml",
            PluginRegistry::new().register::<crate::plugin::ErrorMappingPlugin>(),
        )
        .await
        .expect("Router should initialize successfully");

        wait_for_readiness(&app.app).await;

        let resp = test::call_service(
            &app.app,
            init_graphql_request("{ users { id", None).to_request(),
        )
        .await;

        assert!(resp.status().is_client_error(), "Expected 4xx status code");
        let body = test::read_body(resp).await;
        let body_str = std::str::from_utf8(&body).expect("Response body should be valid UTF-8");
        assert!(
            body_str.contains(r#""code":"InvalidInput""#),
            "Expected error code to be InvalidInput"
        );
    }
}
