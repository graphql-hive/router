#[cfg(test)]
mod subscription_metrics_e2e_tests {
    use std::time::Duration;

    use futures::StreamExt;
    use hive_router_internal::telemetry::metrics::catalog::labels;
    use hive_router_plan_executor::executors::{
        graphql_transport_ws::SubscribePayload, websocket_client::WsClient,
    };
    use ntex::http;

    use crate::testkit::{otel::OtlpCollector, some_header_map, TestRouter, TestSubgraphs};

    async fn wait_for_metrics_export() {
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    const CLIENTS_ACTIVE: &str = "hive.router.subscriptions.clients.active";
    const CLIENTS_CONNECTIONS: &str = "hive.router.subscriptions.clients.connections";
    const CLIENTS_STARTED_TOTAL: &str = "hive.router.subscriptions.clients.started_total";
    const CLIENTS_ENDED_TOTAL: &str = "hive.router.subscriptions.clients.ended_total";
    const CLIENTS_LAGGED_MESSAGES_TOTAL: &str =
        "hive.router.subscriptions.clients.lagged_messages_total";
    const CLIENTS_SENT_MESSAGES_TOTAL: &str =
        "hive.router.subscriptions.clients.sent_messages_total";
    const SUBGRAPHS_ACTIVE: &str = "hive.router.subscriptions.subgraphs.active";
    const SUBGRAPHS_CONNECTIONS: &str = "hive.router.subscriptions.subgraphs.connections";
    const SUBGRAPHS_STARTED_TOTAL: &str = "hive.router.subscriptions.subgraphs.started_total";
    const SUBGRAPHS_ENDED_TOTAL: &str = "hive.router.subscriptions.subgraphs.ended_total";
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

        // reviewAddedLooping never completes on its own, so we control the lifecycle by
        // reading one event (to prove the gauges went up) and then dropping the response
        // stream (to prove the gauges come back down). intervalInMs is high to avoid spamming
        // events while we hold the subscription open.
        let mut res = router
            .send_graphql_request(
                r#"
                subscription {
                    reviewAddedLooping(intervalInMs: 200) {
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
        let _ = res.next().await.expect("expected at least one chunk");

        wait_for_metrics_export().await;
        let metrics = otlp_collector.metrics_view().await;

        let transport_attrs = [(labels::SUBSCRIPTION_TRANSPORT, "http_sse")];
        assert_eq!(
            metrics.latest_counter(CLIENTS_ACTIVE, &transport_attrs),
            1.0,
            "client active gauge must be 1 while the subscription is live"
        );
        assert_eq!(
            metrics.latest_counter(CLIENTS_CONNECTIONS, &transport_attrs),
            1.0,
            "client connections gauge must be 1 while the subscription is live"
        );

        // dropping the response stream tears down the client's RAII guards
        drop(res);

        wait_for_metrics_export().await;
        let metrics = otlp_collector.metrics_view().await;

        assert_eq!(
            metrics.latest_counter(CLIENTS_ACTIVE, &transport_attrs),
            0.0,
            "client active gauge must return to 0 after the subscription ends"
        );
        assert_eq!(
            metrics.latest_counter(CLIENTS_CONNECTIONS, &transport_attrs),
            0.0,
            "client connections gauge must return to 0 after the subscription ends"
        );

        assert_eq!(
            metrics.latest_counter(CLIENTS_STARTED_TOTAL, &transport_attrs),
            1.0,
            "expected exactly one client subscription started event"
        );
        assert_eq!(
            metrics.latest_counter(CLIENTS_ENDED_TOTAL, &transport_attrs),
            1.0,
            "expected exactly one client subscription ended event, matching the start"
        );
        assert_eq!(
            metrics.latest_counter(
                CLIENTS_ENDED_TOTAL,
                &[
                    (labels::SUBSCRIPTION_TRANSPORT, "http_sse"),
                    (labels::SUBSCRIPTION_END_REASON, "client_disconnected"),
                ]
            ),
            1.0,
            "dropping the response stream should be attributed to client_disconnected"
        );

        let subgraph_attrs = [(labels::SUBGRAPH_NAME, "reviews")];
        assert_eq!(
            metrics.latest_counter(SUBGRAPHS_ACTIVE, &subgraph_attrs),
            0.0,
            "subgraph active gauge must return to 0 after the subscription completes"
        );

        assert_eq!(
            metrics.latest_counter(SUBGRAPHS_STARTED_TOTAL, &subgraph_attrs),
            1.0,
            "expected exactly one subgraph subscription started event"
        );
        assert_eq!(
            metrics.latest_counter(SUBGRAPHS_ENDED_TOTAL, &subgraph_attrs),
            1.0,
            "expected exactly one subgraph subscription ended event, matching the start"
        );

        // no lag or drop counters should ever fire on this happy path
        assert_eq!(
            metrics.latest_counter(CLIENTS_LAGGED_MESSAGES_TOTAL, &transport_attrs),
            0.0,
            "lag counter must not fire on normal completion"
        );

        // reviewAddedLooping keeps emitting in the background regardless of how many
        // chunks the client actually read, so only assert delivery happened at all
        assert!(
            metrics.latest_counter(CLIENTS_SENT_MESSAGES_TOTAL, &transport_attrs) >= 1.0,
            "expected at least one message sent to the client"
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

        assert_eq!(
            metrics.latest_counter(CLIENTS_SENT_MESSAGES_TOTAL, &ws_attrs),
            received as f64,
            "expected the sent counter to match the number of events received over the finite WS stream"
        );
        assert_eq!(
            metrics.latest_counter(
                CLIENTS_ENDED_TOTAL,
                &[
                    (labels::SUBSCRIPTION_TRANSPORT, "websocket"),
                    (labels::SUBSCRIPTION_END_REASON, "completed"),
                ]
            ),
            1.0,
            "a finite WS subscription that drains naturally should end with reason completed"
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
            assert!(
                !metrics.has_counter(CLIENTS_SENT_MESSAGES_TOTAL, &attrs),
                "did not expect a {phantom_transport}-labeled client sent series from a WS subscription"
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

        // reviewAdded keeps emitting in the background regardless of how many chunks each
        // client actually read, so only assert both joiners independently received deliveries
        assert!(
            metrics.latest_counter(
                CLIENTS_SENT_MESSAGES_TOTAL,
                &[(labels::SUBSCRIPTION_TRANSPORT, "http_sse")]
            ) >= 1.0,
            "expected at least one message sent to the SSE joiner"
        );
        assert!(
            metrics.latest_counter(
                CLIENTS_SENT_MESSAGES_TOTAL,
                &[(labels::SUBSCRIPTION_TRANSPORT, "http_multipart")]
            ) >= 1.0,
            "expected at least one message sent to the multipart joiner, deduplication must not merge per-client delivery counts"
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
    async fn http_callback_subgraph_transport_metrics() {
        let otlp_collector = OtlpCollector::start()
            .await
            .expect("Failed to start OTLP collector");

        let subgraphs = TestSubgraphs::builder().build().start().await;

        let router_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let router_port = router_listener.local_addr().unwrap().port();
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .with_listener(router_listener)
            .inline_config(format!(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                subscriptions:
                    enabled: true
                    callback:
                        public_url: http://0.0.0.0:{router_port}/callback
                        subgraphs:
                            - reviews
                {}
                "#,
                otlp_metrics_config(&otlp_collector.http_metrics_endpoint())
            ))
            .build()
            .start()
            .await;

        // reviewAddedLooping never completes on its own, so the subscription stays active
        // until we drop the response stream below
        let mut res = router
            .send_graphql_request(
                r#"
                subscription {
                    reviewAddedLooping(intervalInMs: 200) {
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
        let _ = res.next().await.expect("expected at least one chunk");

        wait_for_metrics_export().await;
        let metrics = otlp_collector.metrics_view().await;

        let subgraph_connection_attrs = [
            (labels::SUBGRAPH_NAME, "reviews"),
            (labels::SUBSCRIPTION_TRANSPORT, "http_callback"),
        ];
        assert_eq!(
            metrics.latest_counter(SUBGRAPHS_CONNECTIONS, &subgraph_connection_attrs),
            1.0,
            "expected the http_callback-transported subgraph connection to be counted active"
        );

        let subgraph_attrs = [(labels::SUBGRAPH_NAME, "reviews")];
        assert_eq!(
            metrics.latest_counter(SUBGRAPHS_ACTIVE, &subgraph_attrs),
            1.0,
            "expected the subgraph subscription to be counted active"
        );

        assert_eq!(
            metrics.latest_counter(SUBGRAPHS_STARTED_TOTAL, &subgraph_attrs),
            1.0,
            "expected exactly one subgraph subscription started event"
        );

        // the client itself talks SSE to the router, the callback transport is only used
        // between the router and the subgraph
        let client_attrs = [(labels::SUBSCRIPTION_TRANSPORT, "http_sse")];
        assert_eq!(
            metrics.latest_counter(CLIENTS_ACTIVE, &client_attrs),
            1.0,
            "client active gauge must be 1 while the subscription is live"
        );

        drop(res);

        wait_for_metrics_export().await;
        let metrics = otlp_collector.metrics_view().await;

        assert_eq!(
            metrics.latest_counter(SUBGRAPHS_ACTIVE, &subgraph_attrs),
            0.0,
            "subgraph active gauge must return to 0 after the subscription ends"
        );
        assert_eq!(
            metrics.latest_counter(SUBGRAPHS_CONNECTIONS, &subgraph_connection_attrs),
            0.0,
            "http_callback subgraph connection gauge must return to 0 after the subscription ends"
        );

        assert_eq!(
            metrics.latest_counter(SUBGRAPHS_ENDED_TOTAL, &subgraph_attrs),
            1.0,
            "expected exactly one http_callback subgraph subscription ended event, matching the start"
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
