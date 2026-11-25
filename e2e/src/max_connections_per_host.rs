#[cfg(test)]
mod max_connections_per_host_e2e_tests {
    use std::time::Duration;

    use futures::future::join_all;
    use hive_router::telemetry::metrics::catalog::{labels, labels_for, names};

    use crate::testkit::{otel::OtlpCollector, ClientResponseExt, TestRouter, TestSubgraphs};

    const QUERY: &str = "{ users { id } }";

    /// Router config with metrics exported every 30ms and subgraph dedupe off, so concurrent
    /// identical requests each need their own slot instead of joining one in-flight request.
    fn config(otlp_endpoint: &str, traffic_shaping: &str) -> String {
        format!(
            r#"
            supergraph:
                source: file
                path: supergraph.graphql
            telemetry:
                metrics:
                    exporters:
                        - kind: otlp
                          endpoint: {otlp_endpoint}
                          protocol: http
                          interval: 30ms
                          max_export_timeout: 50ms
            traffic_shaping:
{traffic_shaping}
            "#
        )
    }

    #[ntex::test]
    async fn should_report_requests_waiting_for_a_slot() {
        let otlp_collector = OtlpCollector::start()
            .await
            .expect("Failed to start OTLP collector");
        let subgraphs = TestSubgraphs::builder()
            .with_delay(Duration::from_secs(1))
            .build()
            .start()
            .await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(config(
                &otlp_collector.http_metrics_endpoint(),
                r#"
                max_connections_per_host: 1
                all:
                    dedupe_enabled: false
                    request_timeout: 10s
                "#,
            ))
            .build()
            .start()
            .await;

        let requests = join_all((0..3).map(|_| router.send_graphql_request(QUERY, None, None)));
        let while_waiting = async {
            // one request holds the only slot for 1s, the other two wait for it
            tokio::time::sleep(Duration::from_millis(400)).await;
            otlp_collector.metrics_view().await
        };
        let (responses, metrics) = tokio::join!(requests, while_waiting);

        let attrs = [(labels::SUBGRAPH_NAME, "accounts")];
        assert_eq!(
            metrics.latest_counter(names::SUBGRAPH_REQUESTS_PENDING, &attrs),
            2.0
        );
        // the exported series also carry resource attributes such as `service.name`
        let actual_labels = metrics.latest_attribute_names(names::SUBGRAPH_REQUESTS_PENDING);
        for label in labels_for(names::SUBGRAPH_REQUESTS_PENDING).unwrap() {
            assert!(
                actual_labels.contains(*label),
                "missing {label} in {actual_labels:?}"
            );
        }

        for response in responses {
            assert!(response.status().is_success());
            let body = response.json_body_string_pretty().await;
            assert!(!body.contains("errors"), "{body}");
        }

        tokio::time::sleep(Duration::from_millis(200)).await;
        let metrics = otlp_collector.metrics_view().await;
        assert_eq!(
            metrics.latest_counter(names::SUBGRAPH_REQUESTS_PENDING, &attrs),
            0.0
        );
    }

    #[ntex::test]
    async fn should_stop_counting_requests_that_time_out_waiting_for_a_slot() {
        let otlp_collector = OtlpCollector::start()
            .await
            .expect("Failed to start OTLP collector");
        let subgraphs = TestSubgraphs::builder()
            .with_delay(Duration::from_secs(2))
            .build()
            .start()
            .await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(config(
                &otlp_collector.http_metrics_endpoint(),
                r#"
                max_connections_per_host: 1
                all:
                    dedupe_enabled: false
                    request_timeout: 500ms
                "#,
            ))
            .build()
            .start()
            .await;

        let requests = join_all((0..3).map(|_| router.send_graphql_request(QUERY, None, None)));
        let while_waiting = async {
            tokio::time::sleep(Duration::from_millis(250)).await;
            otlp_collector.metrics_view().await
        };
        let (responses, metrics) = tokio::join!(requests, while_waiting);

        let attrs = [(labels::SUBGRAPH_NAME, "accounts")];
        assert_eq!(
            metrics.latest_counter(names::SUBGRAPH_REQUESTS_PENDING, &attrs),
            2.0
        );

        let mut slot_timeouts = 0;
        for response in responses {
            let body = response.json_body_string_pretty().await;
            if body.contains("SUBGRAPH_REQUEST_SLOT_TIMEOUT") {
                slot_timeouts += 1;
            }
        }
        assert!(slot_timeouts >= 1, "expected a request to time out waiting");

        tokio::time::sleep(Duration::from_millis(200)).await;
        let metrics = otlp_collector.metrics_view().await;
        assert_eq!(
            metrics.latest_counter(names::SUBGRAPH_REQUESTS_PENDING, &attrs),
            0.0
        );
    }

    /// The router's own `request_timeout` drops the whole request while it waits for a slot,
    /// so the wait ends without a slot or a slot timeout.
    #[ntex::test]
    async fn should_stop_counting_requests_cancelled_while_waiting_for_a_slot() {
        let otlp_collector = OtlpCollector::start()
            .await
            .expect("Failed to start OTLP collector");
        let subgraphs = TestSubgraphs::builder()
            .with_delay(Duration::from_secs(5))
            .build()
            .start()
            .await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(config(
                &otlp_collector.http_metrics_endpoint(),
                r#"
                max_connections_per_host: 1
                router:
                    request_timeout: 500ms
                all:
                    dedupe_enabled: false
                    request_timeout: 30s
                "#,
            ))
            .build()
            .start()
            .await;

        let requests = join_all((0..3).map(|_| router.send_graphql_request(QUERY, None, None)));
        let while_waiting = async {
            tokio::time::sleep(Duration::from_millis(250)).await;
            otlp_collector.metrics_view().await
        };
        let (responses, metrics) = tokio::join!(requests, while_waiting);

        let attrs = [(labels::SUBGRAPH_NAME, "accounts")];
        assert_eq!(
            metrics.latest_counter(names::SUBGRAPH_REQUESTS_PENDING, &attrs),
            2.0
        );
        for response in responses {
            assert_eq!(response.status(), ntex::http::StatusCode::GATEWAY_TIMEOUT);
        }

        tokio::time::sleep(Duration::from_millis(200)).await;
        let metrics = otlp_collector.metrics_view().await;
        assert_eq!(
            metrics.latest_counter(names::SUBGRAPH_REQUESTS_PENDING, &attrs),
            0.0
        );
    }

    #[ntex::test]
    async fn should_not_report_pending_requests_under_the_limit() {
        let otlp_collector = OtlpCollector::start()
            .await
            .expect("Failed to start OTLP collector");
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(config(
                &otlp_collector.http_metrics_endpoint(),
                r#"
                max_connections_per_host: 10
                all:
                    dedupe_enabled: false
                "#,
            ))
            .build()
            .start()
            .await;

        let responses =
            join_all((0..3).map(|_| router.send_graphql_request(QUERY, None, None))).await;
        for response in responses {
            assert!(response.status().is_success());
        }

        tokio::time::sleep(Duration::from_millis(200)).await;
        let metrics = otlp_collector.metrics_view().await;
        // the subgraph requests were exported, so the pending metric would have been too
        assert!(metrics.has_histogram(
            names::HTTP_CLIENT_REQUEST_DURATION,
            &[(labels::SUBGRAPH_NAME, "accounts")]
        ));
        assert!(!metrics.has_counter(names::SUBGRAPH_REQUESTS_PENDING, &[]));
    }
}
