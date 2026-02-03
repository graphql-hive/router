#[cfg(test)]
mod websocket_e2e_tests {
    use futures::StreamExt;
    use serde_json::json;
    use std::collections::HashMap;

    use crate::testkit_v2::TestRouterBuilder;
    use hive_router_plan_executor::executors::{
        graphql_transport_ws::ConnectionInitPayload, websocket_client::WsClient,
    };

    #[ntex::test]
    async fn query_over_websocket() {
        let router = TestRouterBuilder::new()
            .with_subgraphs()
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
            .await
            .expect("Failed to start test router");

        let wsconn = router.ws().await;

        let mut client = WsClient::init(wsconn, None)
            .await
            .expect("Failed to init WsClient");

        let mut stream = client
            .subscribe(
                r#"
                query {
                    topProducts {
                        name
                        upc
                    }
                }
                "#
                .to_string(),
                None,
                None,
                None,
            )
            .await;

        let response = stream.next().await.expect("Expected a response");

        assert!(response.errors.is_none(), "Expected no errors");
        assert!(!response.data.is_null(), "Expected data");

        let next = stream.next().await;
        assert!(next.is_none(), "Expected stream to complete after query");
    }

    #[ntex::test]
    async fn subscription_over_websocket() {
        let router = TestRouterBuilder::new()
            .with_subgraphs()
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
            .await
            .expect("Failed to start test router");

        let wsconn = router.ws().await;

        let mut client = WsClient::init(wsconn, None)
            .await
            .expect("Failed to init WsClient");

        let mut stream = client
            .subscribe(
                r#"
                subscription {
                    reviewAdded(step: 1, intervalInMs: 0) {
                        id
                        body
                    }
                }
                "#
                .to_string(),
                None,
                None,
                None,
            )
            .await;

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
        let router = TestRouterBuilder::new()
            .with_subgraphs()
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
            .await
            .expect("Failed to start test router");

        let wsconn = router.ws().await;

        let mut client = WsClient::init(wsconn, None)
            .await
            .expect("Failed to init WsClient");

        let mut stream1 = client
            .subscribe(
                r#"
                subscription {
                    reviewAdded(step: 1, intervalInMs: 0) {
                        id
                    }
                }
                "#
                .to_string(),
                None,
                None,
                None,
            )
            .await;

        let mut stream2 = client
            .subscribe(
                r#"
                subscription {
                    reviewAdded(step: 2, intervalInMs: 0) {
                        id
                    }
                }
                "#
                .to_string(),
                None,
                None,
                None,
            )
            .await;

        let mut count1 = 0;
        let mut count2 = 0;

        loop {
            tokio::select! {
                maybe_response = stream1.next() => {
                    match maybe_response {
                        Some(response) => {
                            assert!(response.errors.is_none(), "Expected no errors in stream1");
                            count1 += 1;
                        }
                        None => {
                            if count2 > 0 {
                                break;
                            }
                        }
                    }
                }
                maybe_response = stream2.next() => {
                    match maybe_response {
                        Some(response) => {
                            assert!(response.errors.is_none(), "Expected no errors in stream2");
                            count2 += 1;
                        }
                        None => {
                            if count1 > 0 {
                                break;
                            }
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
        let router = TestRouterBuilder::new()
            .with_subgraphs()
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
                    # default headers_in_connection_init_payload: true
                "#,
            )
            .build()
            .start()
            .await
            .expect("Failed to start test router");

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

        let mut stream = client
            .subscribe(
                r#"
                query {
                    topProducts {
                        name
                        upc
                    }
                }
                "#
                .to_string(),
                None,
                None,
                None,
            )
            .await;

        stream.next().await.expect("Expected a response");

        let products_requests = router
            .get_subgraph_requests_log("products")
            .await
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
        let router = TestRouterBuilder::new()
            .with_subgraphs()
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
                    headers_in_operation_extensions: true
                "#,
            )
            .build()
            .start()
            .await
            .expect("Failed to start test router");

        let wsconn = router.ws().await;

        let mut client = WsClient::init(wsconn, None)
            .await
            .expect("Failed to init WsClient");

        let mut stream = client
            .subscribe(
                r#"
                query {
                    topProducts {
                        name
                        upc
                    }
                }
                "#
                .to_string(),
                None,
                None,
                Some(HashMap::from([(
                    "x-context".to_string(),
                    json!("my-extensions-value"),
                )])),
            )
            .await;

        stream.next().await.expect("Expected a response");

        let products_requests = router
            .get_subgraph_requests_log("products")
            .await
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
}
