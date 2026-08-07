#[cfg(test)]
mod websocket_e2e_tests {
    use futures::StreamExt;
    use sonic_rs::json;
    use std::collections::HashMap;

    use crate::testkit::{TestRouter, TestSubgraphs};
    use hive_router::{
        async_trait,
        http::StatusCode,
        plugins::{
            hooks::{
                on_graphql_params::{
                    OnGraphQLParamsStartHookPayload, OnGraphQLParamsStartHookResult,
                },
                on_plugin_init::{OnPluginInitPayload, OnPluginInitResult},
            },
            plugin_trait::{EndHookPayload, RouterPlugin, StartHookPayload},
        },
        GraphQLError,
    };
    use hive_router_plan_executor::executors::{
        graphql_transport_ws::{ConnectionInitPayload, SubscribePayload},
        websocket_client::WsClient,
    };

    #[derive(Default)]
    struct TestWebSocketGraphqlParamsPlugin;

    #[async_trait]
    impl RouterPlugin for TestWebSocketGraphqlParamsPlugin {
        type Config = ();

        fn plugin_name() -> &'static str {
            "test_websocket_graphql_params"
        }

        fn on_plugin_init(payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
            payload.initialize_plugin_with_defaults()
        }

        async fn on_graphql_params<'exec>(
            &'exec self,
            mut payload: OnGraphQLParamsStartHookPayload<'exec>,
        ) -> OnGraphQLParamsStartHookResult<'exec> {
            assert_eq!(payload.router_http_request.method, http::Method::POST);
            assert_eq!(payload.router_http_request.path, "/graphql");
            let graphql_params = payload
                .graphql_params
                .as_mut()
                .expect("Expected decoded WebSocket GraphQL parameters");
            graphql_params.query = Some(
                "query SelectedByHook { topProducts { name } } query Other { __typename }"
                    .to_string(),
            );
            payload.on_end(|mut payload| {
                payload.graphql_params.operation_name = Some("SelectedByHook".to_string());
                payload.proceed()
            })
        }
    }

    #[derive(Default)]
    struct TestWebSocketGraphqlParamsEarlyResponsePlugin;

    #[async_trait]
    impl RouterPlugin for TestWebSocketGraphqlParamsEarlyResponsePlugin {
        type Config = ();

        fn plugin_name() -> &'static str {
            "test_websocket_graphql_params_early_response"
        }

        fn on_plugin_init(payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
            payload.initialize_plugin_with_defaults()
        }

        async fn on_graphql_params<'exec>(
            &'exec self,
            payload: OnGraphQLParamsStartHookPayload<'exec>,
        ) -> OnGraphQLParamsStartHookResult<'exec> {
            payload.end_with_graphql_error(
                GraphQLError::from_message_and_code(
                    "Rejected by GraphQL parameters hook",
                    "GRAPHQL_PARAMS_REJECTED",
                ),
                StatusCode::BAD_REQUEST,
            )
        }
    }

    #[ntex::test]
    async fn query_over_websocket() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                websocket:
                    enabled: true
                "#,
            )
            .build()
            .start()
            .await;

        let wsconn = router.ws().await;

        let mut client = WsClient::init(wsconn, None)
            .await
            .expect("Failed to init WsClient");

        let execution_request = SubscribePayload {
            query: r#"
                query {
                    topProducts {
                        name
                        upc
                    }
                }
                "#
            .into(),
            ..Default::default()
        };

        let mut stream = client.subscribe(execution_request, None).await;

        let response = stream.next().await.expect("Expected a response");

        assert!(response.errors.is_none(), "Expected no errors");
        assert!(!response.data.is_null(), "Expected data");

        let next = stream.next().await;
        assert!(next.is_none(), "Expected stream to complete after query");
    }

    #[ntex::test]
    async fn persisted_document_over_websocket() {
        let document_id = "sha256:abc123";
        let manifest =
            tempfile::NamedTempFile::new().expect("Failed to create persisted document manifest");
        std::fs::write(
            manifest.path(),
            sonic_rs::to_string(&json!({
                document_id: "{ topProducts { name } }",
            }))
            .expect("Failed to serialize persisted document manifest"),
        )
        .expect("Failed to write persisted document manifest");

        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(format!(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                websocket:
                    enabled: true
                persisted_documents:
                    enabled: true
                    require_id: true
                    storage:
                        type: file
                        path: "{}"
                "#,
                manifest.path().display(),
            ))
            .build()
            .start()
            .await;

        let wsconn = router.ws().await;
        let mut client = WsClient::init(wsconn, None)
            .await
            .expect("Failed to init WsClient");
        let mut stream = client
            .subscribe(
                SubscribePayload {
                    query: String::new(),
                    extensions: Some(HashMap::from([(
                        "persistedQuery".to_string(),
                        json!({ "sha256Hash": document_id }),
                    )])),
                    ..Default::default()
                },
                None,
            )
            .await;

        let response = stream.next().await.expect("Expected a response");
        assert!(response.errors.is_none(), "Expected no errors");
        assert!(!response.data.is_null(), "Expected data");
        assert!(stream.next().await.is_none(), "Expected stream to complete");
    }

    #[ntex::test]
    async fn graphql_params_hooks_prepare_websocket_operation() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                websocket:
                    enabled: true
                plugins:
                    test_websocket_graphql_params:
                        enabled: true
                "#,
            )
            .register_plugin::<TestWebSocketGraphqlParamsPlugin>()
            .build()
            .start()
            .await;

        let wsconn = router.ws().await;
        let mut client = WsClient::init(wsconn, None)
            .await
            .expect("Failed to init WsClient");
        let mut stream = client
            .subscribe(
                SubscribePayload {
                    query: "not valid GraphQL".to_string(),
                    ..Default::default()
                },
                None,
            )
            .await;

        let response = stream.next().await.expect("Expected a response");
        assert!(response.errors.is_none(), "Expected no errors");
        assert!(!response.data.is_null(), "Expected data");
        assert!(stream.next().await.is_none(), "Expected stream to complete");
    }

    #[ntex::test]
    async fn graphql_params_hook_early_response_completes_websocket_operation() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                websocket:
                    enabled: true
                plugins:
                    test_websocket_graphql_params_early_response:
                        enabled: true
                "#,
            )
            .register_plugin::<TestWebSocketGraphqlParamsEarlyResponsePlugin>()
            .build()
            .start()
            .await;

        let wsconn = router.ws().await;
        let mut client = WsClient::init(wsconn, None)
            .await
            .expect("Failed to init WsClient");
        let mut stream = client
            .subscribe(
                SubscribePayload {
                    query: "{ __typename }".to_string(),
                    ..Default::default()
                },
                None,
            )
            .await;

        let response = stream.next().await.expect("Expected an early response");
        let errors = response.errors.expect("Expected GraphQL errors");
        assert_eq!(
            errors[0].extensions.code.as_deref(),
            Some("GRAPHQL_PARAMS_REJECTED")
        );
        assert!(stream.next().await.is_none(), "Expected stream to complete");
    }

    #[ntex::test]
    async fn subscription_over_websocket() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                websocket:
                    enabled: true
                subscriptions:
                    enabled: true
                "#,
            )
            .build()
            .start()
            .await;

        let wsconn = router.ws().await;

        let mut client = WsClient::init(wsconn, None)
            .await
            .expect("Failed to init WsClient");

        let subscribe_payload = SubscribePayload {
            query: r#"
                subscription {
                    reviewAdded(step: 1, intervalInMs: 0) {
                        id
                        body
                    }
                }
                "#
            .into(),
            ..Default::default()
        };

        let mut stream = client.subscribe(subscribe_payload, None).await;

        let mut received_count = 0;
        while let Some(response) = stream.next().await {
            assert!(response.errors.is_none(), "Expected no errors");
            assert!(!response.data.is_null(), "Expected data");
            received_count += 1;
        }

        assert_eq!(
            received_count, 11,
            "Expected to receive 11 subscription events"
        );
    }

    #[ntex::test]
    async fn multiple_subscriptions_in_parallel() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                websocket:
                    enabled: true
                subscriptions:
                    enabled: true
                "#,
            )
            .build()
            .start()
            .await;

        let wsconn = router.ws().await;

        let mut client = WsClient::init(wsconn, None)
            .await
            .expect("Failed to init WsClient");

        let subscribe_payload1 = SubscribePayload {
            query: r#"
                subscription {
                    reviewAdded(step: 1, intervalInMs: 0) {
                        id
                    }
                }
                "#
            .into(),
            ..Default::default()
        };

        let mut stream1 = client.subscribe(subscribe_payload1, None).await;

        let subscribe_payload = SubscribePayload {
            query: r#"
                subscription {
                    reviewAdded(step: 2, intervalInMs: 0) {
                        id
                    }
                }
                "#
            .into(),
            ..Default::default()
        };

        let mut stream2 = client.subscribe(subscribe_payload, None).await;

        let mut count1 = 0;
        let mut count2 = 0;
        let mut done1 = false;
        let mut done2 = false;

        loop {
            if done1 && done2 {
                break;
            }

            tokio::select! {
                maybe_response = stream1.next(), if !done1 => {
                    match maybe_response {
                        Some(response) => {
                            assert!(response.errors.is_none(), "Expected no errors in stream1");
                            count1 += 1;
                        }
                        None => {
                            done1 = true;
                        }
                    }
                }
                maybe_response = stream2.next(), if !done2 => {
                    match maybe_response {
                        Some(response) => {
                            assert!(response.errors.is_none(), "Expected no errors in stream2");
                            count2 += 1;
                        }
                        None => {
                            done2 = true;
                        }
                    }
                }
            }

            if count1 > 0 && count2 > 0 {
                break;
            }
        }

        assert!(
            count1 > 0,
            "Expected to receive at least one event from stream1"
        );
        assert!(
            count2 > 0,
            "Expected to receive at least one event from stream2"
        );
    }

    #[ntex::test]
    async fn header_propagation_from_connection_init_payload() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                headers:
                    all:
                        request:
                            - propagate:
                                named: x-context
                websocket:
                    enabled: true
                    # default headers.source: connection
                "#,
            )
            .build()
            .start()
            .await;

        let wsconn = router.ws().await;

        let mut client = WsClient::init(
            wsconn,
            Some(ConnectionInitPayload::new(HashMap::from([(
                "x-context".to_string(),
                json!("my-init_payload-value"),
            )]))),
        )
        .await
        .expect("Failed to init WsClient");

        let subscribe_payload = SubscribePayload {
            query: r#"
                query {
                    topProducts {
                        name
                        upc
                    }
                }
                "#
            .into(),
            ..Default::default()
        };

        let mut stream = client.subscribe(subscribe_payload, None).await;

        stream.next().await.expect("Expected a response");

        let products_requests = subgraphs
            .get_requests_log("products")
            .expect("expected requests sent to products subgraph");
        let last_products_request = products_requests
            .last()
            .expect("expected at least one request to products subgraph");
        assert_eq!(
            last_products_request
                .headers
                .get("x-context")
                .expect("expected x-context header to be present"),
            "my-init_payload-value",
        )
    }

    #[ntex::test]
    async fn header_propagation_from_operation_extensions() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                headers:
                    all:
                        request:
                            - propagate:
                                named: x-context
                websocket:
                    enabled: true
                    headers:
                        source: operation
                "#,
            )
            .build()
            .start()
            .await;

        let wsconn = router.ws().await;

        let mut client = WsClient::init(wsconn, None)
            .await
            .expect("Failed to init WsClient");

        let subscribe_payload = SubscribePayload {
            query: r#"
                query {
                    topProducts {
                        name
                        upc
                    }
                }
                "#
            .into(),
            extensions: Some(HashMap::from([(
                "headers".to_string(),
                json!({"x-context": "my-extensions-value"}),
            )])),
            ..Default::default()
        };

        let mut stream = client.subscribe(subscribe_payload, None).await;

        stream.next().await.expect("Expected a response");

        let products_requests = subgraphs
            .get_requests_log("products")
            .expect("expected requests sent to products subgraph");
        let last_products_request = products_requests
            .last()
            .expect("expected at least one request to products subgraph");
        assert_eq!(
            last_products_request
                .headers
                .get("x-context")
                .expect("expected x-context header to be present"),
            "my-extensions-value",
        )
    }

    #[ntex::test]
    async fn merged_header_propagation_from_both_connection_init_payload_and_operation_extensions()
    {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                headers:
                    all:
                        request:
                            - propagate:
                                named: x-context
                websocket:
                    enabled: true
                    headers:
                        source: both
                        persist: true
                "#,
            )
            .build()
            .start()
            .await;

        let wsconn = router.ws().await;

        let mut client = WsClient::init(
            wsconn,
            Some(ConnectionInitPayload::new(HashMap::from([(
                "x-context".to_string(),
                json!("my-init_payload-value"),
            )]))),
        )
        .await
        .expect("Failed to init WsClient");

        let subscribe_payload = SubscribePayload {
            query: r#"
                query {
                    topProducts {
                        name
                        upc
                    }
                }
            "#
            .into(),
            extensions: Some(HashMap::from([(
                "headers".to_string(),
                json!({"x-context": "my-extensions-value"}),
            )])),
            ..Default::default()
        };

        // merging headers
        let mut stream = client.subscribe(subscribe_payload, None).await;
        stream.next().await.expect("Expected a response");

        let subscribe_payload = SubscribePayload {
            query: r#"
                query {
                    topProducts {
                        name
                        upc
                    }
                }
            "#
            .into(),
            ..Default::default()
        };

        // missing headers in extensions, should've been merged
        let mut stream = client.subscribe(subscribe_payload, None).await;
        stream.next().await.expect("Expected a response");

        let products_requests = subgraphs
            .get_requests_log("products")
            .expect("expected requests sent to products subgraph");
        let last_products_request = products_requests
            .last()
            .expect("expected at least one request to products subgraph");
        assert_eq!(
            last_products_request
                .headers
                .get("x-context")
                .expect("expected x-context header to be present"),
            "my-extensions-value",
        )
    }

    #[ntex::test]
    async fn should_not_steal_non_upgrade_get_requests_on_same_graphql_path() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                websocket:
                    enabled: true
                "#,
            )
            .build()
            .start()
            .await;

        let req = router
            .serv()
            .get(router.graphql_path()) // same path as the websocket upgrade endpoint
            .header(http::header::ACCEPT, "application/graphql-response+json")
            .query(&[("query", "{ __typename }")])
            .unwrap();

        let res = req.send().await.unwrap();

        assert_eq!(res.status(), http::StatusCode::OK);
        assert_eq!(
            res.header(http::header::CONTENT_TYPE)
                .expect("expected content-type header to be present"),
            "application/graphql-response+json"
        );
    }
}
