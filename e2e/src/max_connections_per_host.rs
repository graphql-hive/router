#[cfg(test)]
mod max_connections_per_host_e2e_tests {
    use std::time::Duration;

    use futures::future::join_all;
    use hive_router::telemetry::metrics::catalog::{
        labels, labels_for, names, values::SlotWaitResult,
    };

    use crate::testkit::{
        otel::{CollectedMetrics, OtlpCollector},
        ClientResponseExt, TestRouter, TestSubgraphs,
    };

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

    /// How many of the `accounts` waits for a slot ended with `result`, and how long they took.
    fn waits(metrics: &CollectedMetrics, result: SlotWaitResult) -> (u64, f64) {
        metrics.latest_histogram_count_sum(
            names::SUBGRAPH_REQUEST_SLOT_WAIT_DURATION,
            &[
                (labels::SUBGRAPH_NAME, "accounts"),
                (labels::RESULT, result.as_str()),
            ],
        )
    }

    fn assert_has_labels(metrics: &CollectedMetrics, name: &str) {
        // the exported series also carry resource attributes such as `service.name`
        let actual_labels = metrics.latest_attribute_names(name);
        for label in labels_for(name).unwrap() {
            assert!(
                actual_labels.contains(*label),
                "missing {label} on {name} in {actual_labels:?}"
            );
        }
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
        assert_has_labels(&metrics, names::SUBGRAPH_REQUESTS_PENDING);

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

        // the two waiters got the slot after ~1s and ~2s
        let (acquired, seconds) = waits(&metrics, SlotWaitResult::Acquired);
        assert_eq!(acquired, 2);
        assert!((2.5..6.0).contains(&seconds), "{seconds}");
        assert_eq!(waits(&metrics, SlotWaitResult::Timeout).0, 0);
        assert_eq!(waits(&metrics, SlotWaitResult::Cancelled).0, 0);
        assert_has_labels(&metrics, names::SUBGRAPH_REQUEST_SLOT_WAIT_DURATION);
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
                subgraphs:
                    products:
                        request_timeout: 5s
                "#,
            ))
            .build()
            .start()
            .await;

        // every test subgraph is on the same host, so they share the one slot. `products` takes
        // it and keeps it for the 2s the subgraph takes, well past the 500ms `accounts` requests
        // can wait, so both of them have to time out waiting.
        let holder = router.send_graphql_request("{ topProducts { upc } }", None, None);
        let waiters = async {
            // give `products` a head start, so it takes the slot first
            tokio::time::sleep(Duration::from_millis(200)).await;
            let requests = join_all((0..2).map(|_| router.send_graphql_request(QUERY, None, None)));
            let while_waiting = async {
                tokio::time::sleep(Duration::from_millis(250)).await;
                otlp_collector.metrics_view().await
            };
            tokio::join!(requests, while_waiting)
        };
        let (holder, (responses, metrics)) = tokio::join!(holder, waiters);

        let attrs = [(labels::SUBGRAPH_NAME, "accounts")];
        assert_eq!(
            metrics.latest_counter(names::SUBGRAPH_REQUESTS_PENDING, &attrs),
            2.0
        );

        for response in responses {
            let body = response.json_body_string_pretty().await;
            assert!(body.contains("SUBGRAPH_REQUEST_SLOT_TIMEOUT"), "{body}");
        }
        let body = holder.json_body_string_pretty().await;
        assert!(!body.contains("errors"), "{body}");

        tokio::time::sleep(Duration::from_millis(200)).await;
        let metrics = otlp_collector.metrics_view().await;
        assert_eq!(
            metrics.latest_counter(names::SUBGRAPH_REQUESTS_PENDING, &attrs),
            0.0
        );
        let (timed_out, seconds) = waits(&metrics, SlotWaitResult::Timeout);
        assert_eq!(timed_out, 2);
        assert!(seconds >= 0.9, "{seconds}");
        assert_eq!(waits(&metrics, SlotWaitResult::Acquired).0, 0);
        assert_eq!(waits(&metrics, SlotWaitResult::Cancelled).0, 0);
    }

    /// The router's own `request_timeout` drops the whole request while it waits for a slot,
    /// so the wait doesn't end as a slot timeout.
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

        // all three requests share one router deadline, and the slot holder started first, so
        // it's dropped a moment before the waiters. A waiter can take the freed slot just before
        // its own deadline, so a wait may end as `acquired` instead of `cancelled`. Either way
        // it never ends as a slot timeout, and the pending count goes back to 0. The unit test
        // `acquire_slot_stops_counting_a_request_that_is_cancelled_while_waiting` covers the
        // exact count.
        let (cancelled, _) = waits(&metrics, SlotWaitResult::Cancelled);
        let acquired = waits(&metrics, SlotWaitResult::Acquired).0;
        assert_eq!(cancelled + acquired, 2);
        assert_eq!(waits(&metrics, SlotWaitResult::Timeout).0, 0);
    }

    /// `0` turns the limit off: requests to the same host all run at once and never wait.
    // https://github.com/graphql-hive/router/issues/1959
    #[ntex::test]
    async fn should_not_limit_requests_when_max_connections_per_host_is_zero() {
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
                max_connections_per_host: 0
                all:
                    dedupe_enabled: false
                    request_timeout: 5s
                "#,
            ))
            .build()
            .start()
            .await;

        let started = std::time::Instant::now();
        let responses =
            join_all((0..5).map(|_| router.send_graphql_request(QUERY, None, None))).await;
        let elapsed = started.elapsed();

        for response in responses {
            assert!(response.status().is_success());
            let body = response.json_body_string_pretty().await;
            assert!(!body.contains("errors"), "{body}");
        }
        // all five ran at once, not one after another
        assert!(elapsed < Duration::from_secs(3), "{elapsed:?}");

        tokio::time::sleep(Duration::from_millis(200)).await;
        let metrics = otlp_collector.metrics_view().await;
        assert!(metrics.has_histogram(
            names::HTTP_CLIENT_REQUEST_DURATION,
            &[(labels::SUBGRAPH_NAME, "accounts")]
        ));
        assert!(!metrics.has_counter(names::SUBGRAPH_REQUESTS_PENDING, &[]));
        assert!(!metrics.has_histogram(names::SUBGRAPH_REQUEST_SLOT_WAIT_DURATION, &[]));
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
        assert!(!metrics.has_histogram(names::SUBGRAPH_REQUEST_SLOT_WAIT_DURATION, &[]));
    }
}
