#[cfg(test)]
mod subscription_metrics_e2e_tests {
    use std::time::Duration;

    use futures::StreamExt;
    use hive_router_internal::telemetry::metrics::catalog::labels;
    use hive_router_plan_executor::executors::{
        graphql_transport_ws::SubscribePayload, websocket_client::WsClient,
    };
    use ntex::http;

    use crate::testkit::{
        otel::OtlpCollector, some_header_map, ClientResponseExt, TestRouter, TestSubgraphs,
    };

    async fn wait_for_metrics_export() {
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    const CLIENTS_ACTIVE: &str = "hive.router.subscriptions.clients.active";
    const CLIENTS_CONNECTIONS: &str = "hive.router.subscriptions.clients.connections";
    const CLIENTS_OPERATIONS_TOTAL: &str = "hive.router.subscriptions.clients.operations_total";
    const CLIENTS_LAGGED_MESSAGES_TOTAL: &str =
        "hive.router.subscriptions.clients.lagged_messages_total";
    const SUBGRAPHS_ACTIVE: &str = "hive.router.subscriptions.subgraphs.active";
    const SUBGRAPHS_OPERATIONS_TOTAL: &str = "hive.router.subscriptions.subgraphs.operations_total";
    const SUBGRAPHS_DROPPED_MESSAGES_TOTAL: &str =
        "hive.router.subscriptions.subgraphs.dropped_messages_total";

    fn otlp_metrics_config(otlp_endpoint: &str) -> String {
        format!(
            r#"telemetry:
                    metrics:
                        exporters:
                            - kind: otlp
                              endpoint: {otlp_endpoint}
                              protocol: http
                              interval: 30ms
                              max_export_timeout: 2s"#
        )
    }

    #[ntex::test]
    async fn sse_subscription_lifecycle_metrics_return_to_zero() {
        let otlp_collector = OtlpCollector::start()
            .await
            .expect("Failed to start OTLP collector");

        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(format!(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                subscriptions:
                    enabled: true
                {}
                "#,
                otlp_metrics_config(&otlp_collector.http_metrics_endpoint())
            ))
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(
                r#"
                subscription {
                    reviewAdded(intervalInMs: 0) {
                        id
                    }
                }
                "#,
                None,
                some_header_map! {
                    http::header::ACCEPT => "text/event-stream"
                },
            )
            .await;

        assert!(res.status().is_success(), "Expected 200 OK");
        // this query is a finite stream, so draining the body waits for completion
        // and drops the subscription's RAII guards before we assert on the metrics.
        let body = res.string_body().await;
        assert!(body.contains("event: complete"));

        wait_for_metrics_export().await;
        let metrics = otlp_collector.metrics_view().await;

        let transport_attrs = [(labels::SUBSCRIPTION_TRANSPORT, "http_sse")];
        assert_eq!(
            metrics.latest_counter(CLIENTS_ACTIVE, &transport_attrs),
            0.0,
            "client active gauge must return to 0 after the subscription completes"
        );
        assert_eq!(
            metrics.latest_counter(CLIENTS_CONNECTIONS, &transport_attrs),
            0.0,
            "client connections gauge must return to 0 after the subscription completes"
        );

        let subscribe_attrs = [
            (labels::SUBSCRIPTION_TRANSPORT, "http_sse"),
            (labels::SUBSCRIPTION_OPERATION, "subscribe"),
        ];
        let unsubscribe_attrs = [
            (labels::SUBSCRIPTION_TRANSPORT, "http_sse"),
            (labels::SUBSCRIPTION_OPERATION, "unsubscribe"),
        ];
        assert_eq!(
            metrics.latest_counter(CLIENTS_OPERATIONS_TOTAL, &subscribe_attrs),
            1.0,
            "expected exactly one client subscribe event"
        );
        assert_eq!(
            metrics.latest_counter(CLIENTS_OPERATIONS_TOTAL, &unsubscribe_attrs),
            1.0,
            "expected exactly one client unsubscribe event, matching the subscribe"
        );

        let subgraph_attrs = [(labels::SUBGRAPH_NAME, "reviews")];
        assert_eq!(
            metrics.latest_counter(SUBGRAPHS_ACTIVE, &subgraph_attrs),
            0.0,
            "subgraph active gauge must return to 0 after the subscription completes"
        );

        let subgraph_subscribe_attrs = [
            (labels::SUBGRAPH_NAME, "reviews"),
            (labels::SUBSCRIPTION_OPERATION, "subscribe"),
        ];
        let subgraph_unsubscribe_attrs = [
            (labels::SUBGRAPH_NAME, "reviews"),
            (labels::SUBSCRIPTION_OPERATION, "unsubscribe"),
        ];
        assert_eq!(
            metrics.latest_counter(SUBGRAPHS_OPERATIONS_TOTAL, &subgraph_subscribe_attrs),
            1.0,
            "expected exactly one subgraph subscribe event"
        );
        assert_eq!(
            metrics.latest_counter(SUBGRAPHS_OPERATIONS_TOTAL, &subgraph_unsubscribe_attrs),
            1.0,
            "expected exactly one subgraph unsubscribe event, matching the subscribe"
        );

        // no lag or drop counters should ever fire on this happy path
        assert_eq!(
            metrics.latest_counter(CLIENTS_LAGGED_MESSAGES_TOTAL, &transport_attrs),
            0.0,
            "lag counter must not fire on normal completion"
        );
    }

    #[ntex::test]
    async fn websocket_subscription_only_produces_websocket_labeled_series() {
        let otlp_collector = OtlpCollector::start()
            .await
            .expect("Failed to start OTLP collector");

        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(format!(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                subscriptions:
                    enabled: true
                websocket:
                    enabled: true
                {}
                "#,
                otlp_metrics_config(&otlp_collector.http_metrics_endpoint())
            ))
            .build()
            .start()
            .await;

        let query = r#"
            subscription {
                reviewAdded(intervalInMs: 0) {
                    id
                }
            }
        "#;

        let wsconn = router.ws().await;
        let mut ws_client = WsClient::init(wsconn, None)
            .await
            .expect("Failed to init WsClient");
        let ws_payload = SubscribePayload {
            query: query.into(),
            ..Default::default()
        };
        let mut ws_stream = ws_client.subscribe(ws_payload, None).await;

        // drain the finite stream so subscribe/unsubscribe both happen while the router runs
        let mut received = 0;
        while let Some(response) = ws_stream.next().await {
            assert!(response.errors.is_none());
            received += 1;
        }
        assert!(received > 0, "expected at least one event over the WS");
        drop(ws_stream);
        drop(ws_client);

        wait_for_metrics_export().await;
        let metrics = otlp_collector.metrics_view().await;

        let ws_attrs = [(labels::SUBSCRIPTION_TRANSPORT, "websocket")];
        assert_eq!(
            metrics.latest_counter(CLIENTS_ACTIVE, &ws_attrs),
            0.0,
            "expected websocket client active gauge to return to 0"
        );

        for phantom_transport in ["http_sse", "http_multipart", "http_callback"] {
            let attrs = [(labels::SUBSCRIPTION_TRANSPORT, phantom_transport)];
            assert!(
                !metrics.has_counter(CLIENTS_ACTIVE, &attrs),
                "did not expect a {phantom_transport}-labeled client series from a WS subscription"
            );
            assert!(
                !metrics.has_counter(CLIENTS_CONNECTIONS, &attrs),
                "did not expect a {phantom_transport}-labeled client connection series from a WS subscription"
            );
        }
    }

    #[ntex::test]
    async fn deduplicated_clients_are_all_counted_active() {
        let otlp_collector = OtlpCollector::start()
            .await
            .expect("Failed to start OTLP collector");

        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(format!(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                subscriptions:
                    enabled: true
                traffic_shaping:
                    router:
                        dedupe:
                            enabled: true
                            headers: none
                {}
                "#,
                otlp_metrics_config(&otlp_collector.http_metrics_endpoint())
            ))
            .build()
            .start()
            .await;

        let query = r#"
            subscription {
                reviewAdded(intervalInMs: 100) {
                    id
                }
            }
        "#;

        let sse_headers = some_header_map! {
            http::header::ACCEPT => "text/event-stream"
        };
        let multipart_headers = some_header_map! {
            http::header::ACCEPT => "multipart/mixed;subscriptionSpec=1.0"
        };

        let mut sub1 = router.send_graphql_request(query, None, sse_headers).await;
        assert!(sub1.status().is_success());
        let _ = sub1.next().await;

        let mut sub2 = router
            .send_graphql_request(query, None, multipart_headers)
            .await;
        assert!(sub2.status().is_success());
        let _ = sub2.next().await;

        wait_for_metrics_export().await;
        let metrics = otlp_collector.metrics_view().await;

        assert_eq!(
            metrics.latest_counter(
                CLIENTS_ACTIVE,
                &[(labels::SUBSCRIPTION_TRANSPORT, "http_sse")]
            ),
            1.0,
            "expected the SSE joiner to be counted active"
        );
        assert_eq!(
            metrics.latest_counter(
                CLIENTS_ACTIVE,
                &[(labels::SUBSCRIPTION_TRANSPORT, "http_multipart")]
            ),
            1.0,
            "expected the multipart joiner to be counted active, not just the first subscriber"
        );

        // only one subgraph operation was created, even though two clients joined
        assert_eq!(
            metrics.latest_counter(SUBGRAPHS_ACTIVE, &[(labels::SUBGRAPH_NAME, "reviews")]),
            1.0,
            "expected exactly one deduplicated subgraph operation"
        );

        drop(sub1);
        drop(sub2);

        assert_eq!(
            subgraphs
                .get_requests_log("reviews")
                .unwrap_or_default()
                .len(),
            1,
            "requests to reviews subgraph should be deduplicated"
        );
    }

    #[ntex::test]
    async fn backpressure_drops_message_and_keeps_subscription_alive() {
        let otlp_collector = OtlpCollector::start()
            .await
            .expect("Failed to start OTLP collector");

        let subgraphs = TestSubgraphs::builder()
            // delay slows down entity resolution so the subgraph buffer fills up while the
            // subgraph keeps emitting every 10ms
            .with_delay(Duration::from_millis(30))
            .build()
            .start()
            .await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(format!(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                subscriptions:
                    enabled: true
                    subgraph_buffer_capacity: 1
                {}
                "#,
                otlp_metrics_config(&otlp_collector.http_metrics_endpoint())
            ))
            .build()
            .start()
            .await;

        let mut res = router
            .send_graphql_request(
                r#"subscription { reviewAddedLooping(intervalInMs: 10) { id product { name } } }"#,
                None,
                some_header_map! { http::header::ACCEPT => "text/event-stream" },
            )
            .await;
        assert!(res.status().is_success());

        // read one event to confirm the subgraph subscription is established
        let _ = res.next().await.expect("expected at least one chunk");

        // let the buffer fill and trigger backpressure handling
        tokio::time::sleep(Duration::from_millis(150)).await;

        assert_eq!(
            subgraphs.active_subscriptions(),
            1,
            "subscription must survive a full buffer instead of being terminated"
        );

        wait_for_metrics_export().await;
        let metrics = otlp_collector.metrics_view().await;

        // don't pin the transport label here: the test subgraph negotiates multipart vs SSE
        // on its own, we only care that a drop was recorded on whichever HTTP transport was used
        let dropped = metrics.latest_counter(SUBGRAPHS_DROPPED_MESSAGES_TOTAL, &[]);
        assert!(
            dropped > 0.0,
            "expected at least one dropped message to be recorded, got {dropped}"
        );

        // the subgraph operation must still be active, the drop must not have torn it down
        assert_eq!(
            metrics.latest_counter(SUBGRAPHS_ACTIVE, &[(labels::SUBGRAPH_NAME, "reviews")]),
            1.0,
            "subgraph active gauge must still show the subscription as alive"
        );

        drop(res);
    }
}
