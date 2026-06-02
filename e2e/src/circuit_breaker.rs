#[cfg(test)]
mod circuit_breaker_e2e_tests {
    use std::{thread::sleep, time::Duration};

    use hive_router_internal::telemetry::{
        metrics::catalog::{labels, names},
        traces::spans::{
            attributes::{
                ERROR_TYPE, HIVE_ERROR_SUBGRAPH_NAME, HIVE_GRAPHQL_ERROR_CODES,
                HIVE_GRAPHQL_ERROR_COUNT, HIVE_KIND,
            },
            kind::HiveEventKind,
        },
    };

    use crate::testkit::{
        otel::{CollectedSpan, OtlpCollector},
        some_header_map, ClientResponseExt, ResponseLike, TestRouter, TestSubgraphs,
    };

    /// Asserts that the given span carries a `SUBGRAPH_CIRCUIT_BREAKER_REJECTED`
    /// error against the `accounts` subgraph, both via the aggregated
    /// attributes (`hive.graphql.error.codes`, `hive.graphql.error.count`) and
    /// via a `graphql.error` event.
    fn assert_breaker_rejection_recorded_on_span(span: &CollectedSpan, span_label: &str) {
        let codes = span
            .attributes
            .get(HIVE_GRAPHQL_ERROR_CODES)
            .unwrap_or_else(|| panic!("{span_label} span must have hive.graphql.error.codes set"));
        assert!(
            codes.contains("SUBGRAPH_CIRCUIT_BREAKER_REJECTED"),
            "{span_label}: expected SUBGRAPH_CIRCUIT_BREAKER_REJECTED in error codes, got: {codes}"
        );

        let count = span
            .attributes
            .get(HIVE_GRAPHQL_ERROR_COUNT)
            .unwrap_or_else(|| panic!("{span_label} span must have hive.graphql.error.count set"));
        let count: usize = count
            .parse()
            .expect("hive.graphql.error.count must be numeric");
        assert!(
            count >= 1,
            "{span_label}: expected at least one error recorded, got {count}"
        );

        let kind: &'static str = HiveEventKind::GraphQLError.into();
        let breaker_event = span
            .events
            .iter()
            .find(|event| {
                event.attributes.get(HIVE_KIND).map(String::as_str) == Some(kind)
                    && event.attributes.get(ERROR_TYPE).map(String::as_str)
                        == Some("SUBGRAPH_CIRCUIT_BREAKER_REJECTED")
            })
            .unwrap_or_else(|| {
                panic!("{span_label}: expected a graphql.error event for the breaker rejection",)
            });
        assert_eq!(
            breaker_event
                .attributes
                .get(HIVE_ERROR_SUBGRAPH_NAME)
                .map(String::as_str),
            Some("accounts"),
            "{span_label}: graphql.error event must carry the subgraph name attribute"
        );
    }

    #[ntex::test]
    async fn should_open_circuit_breaker_after_slow_subgraph_timeouts() {
        let subgraphs = TestSubgraphs::builder()
            .with_on_request(|request| {
                if request.path == "/accounts" {
                    sleep(Duration::from_millis(700));
                }
                None
            })
            .build()
            .start()
            .await;

        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
        supergraph:
          source: file
          path: supergraph.graphql
        traffic_shaping:
          all:
            request_timeout: 200ms
            circuit_breaker:
              enabled: true
              error_threshold: 50%
              volume_threshold: 3
              reset_timeout: 30s
        "#,
            )
            .build()
            .start()
            .await;

        for _ in 1..=3 {
            let _ = router
                .send_graphql_request("{ users { id } }", None, None)
                .await;
        }

        let res = router
            .send_graphql_request("{ users { id } }", None, None)
            .await;
        assert_eq!(res.status(), ntex::http::StatusCode::OK);
        insta::assert_snapshot!(
            res.json_body_string_pretty().await,
            @r###"{
  "data": {
    "users": null
  },
  "errors": [
    {
      "message": "Request to subgraph timed out",
      "extensions": {
        "code": "SUBGRAPH_REQUEST_TIMEOUT",
        "serviceName": "accounts"
      }
    }
  ]
}"###
        );

        let res = router
            .send_graphql_request("{ users { id } }", None, None)
            .await;
        assert_eq!(res.status(), ntex::http::StatusCode::OK);
        insta::assert_snapshot!(
            res.json_body_string_pretty().await,
            @r###"{
  "data": {
    "users": null
  },
  "errors": [
    {
      "message": "Rejected by the circuit breaker",
      "extensions": {
        "code": "SUBGRAPH_CIRCUIT_BREAKER_REJECTED",
        "serviceName": "accounts"
      }
    }
  ]
}"###
        );
    }

    #[ntex::test]
    async fn should_not_open_circuit_breaker_when_subgraph_timeout_override_allows_request() {
        let subgraphs = TestSubgraphs::builder()
            .with_on_request(|request| {
                if request.path == "/accounts" {
                    sleep(Duration::from_millis(400));
                }
                None
            })
            .build()
            .start()
            .await;

        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
        supergraph:
          source: file
          path: supergraph.graphql
        traffic_shaping:
          all:
            request_timeout: 100ms
            circuit_breaker:
              enabled: true
              error_threshold: 50%
              volume_threshold: 3
              reset_timeout: 30s
          subgraphs:
            accounts:
              request_timeout: 1s
        "#,
            )
            .build()
            .start()
            .await;

        for _ in 1..=4 {
            let res = router
                .send_graphql_request("{ users { id } }", None, None)
                .await;
            assert_eq!(res.status(), ntex::http::StatusCode::OK);
        }

        let res = router
            .send_graphql_request("{ users { id } }", None, None)
            .await;
        assert_eq!(res.status(), ntex::http::StatusCode::OK);
        insta::assert_snapshot!(
            res.json_body_string_pretty().await,
            @r###"{
  "data": {
    "users": [
      {
        "id": "1"
      },
      {
        "id": "2"
      },
      {
        "id": "3"
      },
      {
        "id": "4"
      },
      {
        "id": "5"
      },
      {
        "id": "6"
      }
    ]
  }
}"###
        );
    }

    #[ntex::test]
    async fn should_open_circuit_breaker_after_error_threshold() {
        let mut accounts_server = mockito::Server::new_async().await;
        let host = accounts_server.host_with_port();

        let error_mock = accounts_server
            .mock("POST", "/accounts")
            .with_status(503)
            .expect(4)
            .create_async()
            .await;

        let router = TestRouter::builder()
            .inline_config(&format!(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                traffic_shaping:
                    all:
                        circuit_breaker:
                            enabled: true
                            error_threshold: 50%
                            volume_threshold: 3
                            reset_timeout: 30s
                override_subgraph_urls:
                    subgraphs:
                        accounts:
                            url: "http://{host}/accounts"
                "#
            ))
            .build()
            .start()
            .await;

        for _ in 1..=3 {
            let _ = router
                .send_graphql_request("{ users { id } }", None, None)
                .await;
        }

        let res = router
            .send_graphql_request("{ users { id } }", None, None)
            .await;

        insta::assert_snapshot!(
        &res.json_body_string_pretty().await,
        @r###"{
  "data": {
    "users": null
  },
  "errors": [
    {
      "message": "Received empty response body from subgraph \"accounts\"",
      "extensions": {
        "code": "SUBGRAPH_RESPONSE_BODY_EMPTY",
        "serviceName": "accounts"
      }
    }
  ]
}"###
            );

        error_mock.assert_async().await;

        let res = router
            .send_graphql_request("{ users { id } }", None, None)
            .await;

        assert_eq!(res.status(), ntex::http::StatusCode::OK);

        insta::assert_snapshot!(
            res.json_body_string_pretty().await,
            @r###"{
  "data": {
    "users": null
  },
  "errors": [
    {
      "message": "Rejected by the circuit breaker",
      "extensions": {
        "code": "SUBGRAPH_CIRCUIT_BREAKER_REJECTED",
        "serviceName": "accounts"
      }
    }
  ]
}"###
        );
    }

    #[ntex::test]
    async fn should_close_circuit_breaker_after_reset_timeout() {
        let mut accounts_server = mockito::Server::new_async().await;
        let host = accounts_server.host_with_port();

        let error_mock = accounts_server
            .mock("POST", "/accounts")
            .with_status(503)
            .expect(4)
            .create_async()
            .await;

        let router = TestRouter::builder()
            .inline_config(&format!(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                traffic_shaping:
                    all:
                        circuit_breaker:
                            enabled: true
                            error_threshold: 50%
                            volume_threshold: 3
                            reset_timeout: 2s
                override_subgraph_urls:
                    subgraphs:
                        accounts:
                            url: "http://{host}/accounts"
                "#
            ))
            .build()
            .start()
            .await;

        for _ in 1..=4 {
            let _ = router
                .send_graphql_request("{ users { id } }", None, None)
                .await;
        }

        error_mock.assert_async().await;

        let res = router
            .send_graphql_request("{ users { id } }", None, None)
            .await;
        insta::assert_snapshot!(
            res.json_body_string_pretty().await,
            @r###"{
  "data": {
    "users": null
  },
  "errors": [
    {
      "message": "Rejected by the circuit breaker",
      "extensions": {
        "code": "SUBGRAPH_CIRCUIT_BREAKER_REJECTED",
        "serviceName": "accounts"
      }
    }
  ]
}"###
        );

        let success_mock = accounts_server
            .mock("POST", "/accounts")
            .with_status(200)
            .with_body(r#"{"data":{"users":[{"id":"1"}]}}"#)
            .expect_at_least(1)
            .create_async()
            .await;

        tokio::time::sleep(Duration::from_secs(3)).await;

        let res = router
            .send_graphql_request("{ users { id } }", None, None)
            .await;

        insta::assert_snapshot!(
            res.json_body_string_pretty().await,
            @r###"{
  "data": {
    "users": [
      {
        "id": "1"
      }
    ]
  }
}"###
        );

        success_mock.assert_async().await;
    }

    #[ntex::test]
    async fn should_support_per_subgraph_circuit_breaker_config() {
        let mut accounts_server = mockito::Server::new_async().await;
        let accounts_host = accounts_server.host_with_port();

        let mut products_server = mockito::Server::new_async().await;
        let products_host = products_server.host_with_port();

        let accounts_error_mock = accounts_server
            .mock("POST", "/accounts")
            .with_status(503)
            .expect(3)
            .create_async()
            .await;

        let products_success_mock = products_server
            .mock("POST", "/products")
            .with_status(200)
            .with_body(r#"{"data":{"topProducts":[{"upc":"1"}]}}"#)
            .expect_at_least(1)
            .create_async()
            .await;

        let router = TestRouter::builder()
            .inline_config(&format!(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                traffic_shaping:
                    all:
                        circuit_breaker:
                            enabled: false
                    subgraphs:
                        accounts:
                            circuit_breaker:
                                enabled: true
                                error_threshold: 50%
                                volume_threshold: 2
                                reset_timeout: 30s
                override_subgraph_urls:
                    subgraphs:
                        accounts:
                            url: "http://{accounts_host}/accounts"
                        products:
                            url: "http://{products_host}/products"
                "#
            ))
            .build()
            .start()
            .await;

        for _ in 1..=3 {
            let _ = router
                .send_graphql_request("{ users { id } }", None, None)
                .await;
        }

        accounts_error_mock.assert_async().await;

        let res = router
            .send_graphql_request("{ users { id } }", None, None)
            .await;

        insta::assert_snapshot!(
            res.json_body_string_pretty().await,
            @r###"{
  "data": {
    "users": null
  },
  "errors": [
    {
      "message": "Rejected by the circuit breaker",
      "extensions": {
        "code": "SUBGRAPH_CIRCUIT_BREAKER_REJECTED",
        "serviceName": "accounts"
      }
    }
  ]
}"###
        );

        let res = router
            .send_graphql_request("{ topProducts { upc } }", None, None)
            .await;

        insta::assert_snapshot!(
            res.json_body_string_pretty().await,
            @r###"{
  "data": {
    "topProducts": [
      {
        "upc": "1"
      }
    ]
  }
}"###
        );

        products_success_mock.assert_async().await;
    }

    /// With `volume_threshold: 10` and `error_threshold: 90%`, the breaker
    /// only starts evaluating after the ring buffer of size 10 fills up.
    /// Sending exactly `volume_threshold` failing requests is therefore not
    /// enough on its own to flip the breaker open; the next request still
    /// reaches the subgraph. This guards against accidentally tripping the
    /// breaker too eagerly when the configured volume is barely reached.
    #[ntex::test]
    async fn should_not_open_circuit_breaker_when_volume_threshold_just_reached() {
        let mut accounts_server = mockito::Server::new_async().await;
        let host = accounts_server.host_with_port();

        let mock = accounts_server
            .mock("POST", "/accounts")
            .with_status(503)
            .expect_at_least(1)
            .create_async()
            .await;

        let router = TestRouter::builder()
            .inline_config(&format!(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                traffic_shaping:
                    all:
                        circuit_breaker:
                            enabled: true
                            error_threshold: 90%
                            volume_threshold: 10
                            reset_timeout: 30s
                override_subgraph_urls:
                    subgraphs:
                        accounts:
                            url: "http://{host}/accounts"
                "#
            ))
            .build()
            .start()
            .await;

        for _ in 1..=10 {
            let _ = router
                .send_graphql_request("{ users { id } }", None, None)
                .await;
        }

        let res = router
            .send_graphql_request("{ users { id } }", None, None)
            .await;

        assert_eq!(res.status(), ntex::http::StatusCode::OK);

        insta::assert_snapshot!(
            res.json_body_string_pretty().await,
            @r###"{
  "data": {
    "users": null
  },
  "errors": [
    {
      "message": "Received empty response body from subgraph \"accounts\"",
      "extensions": {
        "code": "SUBGRAPH_RESPONSE_BODY_EMPTY",
        "serviceName": "accounts"
      }
    }
  ]
}"###
        );

        mock.assert_async().await;
    }

    #[ntex::test]
    async fn should_not_activate_circuit_breaker_when_disabled() {
        let mut accounts_server = mockito::Server::new_async().await;
        let host = accounts_server.host_with_port();

        let error_mock = accounts_server
            .mock("POST", "/accounts")
            .with_status(503)
            .expect_at_least(10)
            .create_async()
            .await;

        let router = TestRouter::builder()
            .inline_config(&format!(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                traffic_shaping:
                    all:
                        circuit_breaker:
                            enabled: false
                override_subgraph_urls:
                    subgraphs:
                        accounts:
                            url: "http://{host}/accounts"
                "#
            ))
            .build()
            .start()
            .await;

        for _ in 1..=9 {
            let _ = router
                .send_graphql_request("{ users { id } }", None, None)
                .await;
        }

        let res = router
            .send_graphql_request("{ users { id } }", None, None)
            .await;

        insta::assert_snapshot!(
        &res.json_body_string_pretty().await,
        @r###"{
  "data": {
    "users": null
  },
  "errors": [
    {
      "message": "Received empty response body from subgraph \"accounts\"",
      "extensions": {
        "code": "SUBGRAPH_RESPONSE_BODY_EMPTY",
        "serviceName": "accounts"
      }
    }
  ]
}"###
            );

        error_mock.assert_async().await;
    }

    #[ntex::test]
    async fn should_override_global_circuit_breaker_with_subgraph_config() {
        let mut accounts_server = mockito::Server::new_async().await;
        let host = accounts_server.host_with_port();

        let error_mock = accounts_server
            .mock("POST", "/accounts")
            .with_status(503)
            .expect(6)
            .create_async()
            .await;

        let router = TestRouter::builder()
            .inline_config(&format!(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                traffic_shaping:
                    all:
                        circuit_breaker:
                            enabled: true
                            error_threshold: 50%
                            volume_threshold: 3
                            reset_timeout: 30s
                    subgraphs:
                        accounts:
                            circuit_breaker:
                                enabled: true
                                volume_threshold: 5
                override_subgraph_urls:
                    subgraphs:
                        accounts:
                            url: "http://{host}/accounts"
                "#
            ))
            .build()
            .start()
            .await;

        for _ in 1..=6 {
            let _ = router
                .send_graphql_request("{ users { id } }", None, None)
                .await;
        }

        error_mock.assert_async().await;

        let res = router
            .send_graphql_request("{ users { id } }", None, None)
            .await;

        insta::assert_snapshot!(
            res.json_body_string_pretty().await,
            @r###"{
  "data": {
    "users": null
  },
  "errors": [
    {
      "message": "Rejected by the circuit breaker",
      "extensions": {
        "code": "SUBGRAPH_CIRCUIT_BREAKER_REJECTED",
        "serviceName": "accounts"
      }
    }
  ]
}"###
        );

        error_mock.assert_async().await;
    }

    /// Requests to an unreachable upstream time out and surface as
    /// `SUBGRAPH_REQUEST_TIMEOUT`. Those timeouts must be counted as failures
    /// by the circuit breaker so that, after the threshold is reached, further
    /// requests are short-circuited with `SUBGRAPH_CIRCUIT_BREAKER_REJECTED`.
    #[ntex::test]
    async fn should_open_circuit_breaker_on_subgraph_unreachable_timeouts() {
        let non_existent_host = "192.0.2.1:9999";

        let router = TestRouter::builder()
            .inline_config(&format!(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                traffic_shaping:
                    all:
                        request_timeout: 500ms
                        circuit_breaker:
                            enabled: true
                            error_threshold: 50%
                            volume_threshold: 3
                            reset_timeout: 30s
                override_subgraph_urls:
                    subgraphs:
                        accounts:
                            url: "http://{non_existent_host}/accounts"
                "#
            ))
            .build()
            .start()
            .await;

        for _ in 1..=3 {
            let _ = router
                .send_graphql_request("{ users { id } }", None, None)
                .await;
        }

        let res = router
            .send_graphql_request("{ users { id } }", None, None)
            .await;

        insta::assert_snapshot!(
        &res.json_body_string_pretty().await,
        @r###"{
  "data": {
    "users": null
  },
  "errors": [
    {
      "message": "Request to subgraph timed out",
      "extensions": {
        "code": "SUBGRAPH_REQUEST_TIMEOUT",
        "serviceName": "accounts"
      }
    }
  ]
}"###
            );

        let res = router
            .send_graphql_request("{ users { id } }", None, None)
            .await;

        insta::assert_snapshot!(
            res.json_body_string_pretty().await,
            @r###"{
  "data": {
    "users": null
  },
  "errors": [
    {
      "message": "Rejected by the circuit breaker",
      "extensions": {
        "code": "SUBGRAPH_CIRCUIT_BREAKER_REJECTED",
        "serviceName": "accounts"
      }
    }
  ]
}"###
        );
    }

    #[ntex::test]
    async fn should_record_short_circuit_metrics() {
        let otlp_collector = OtlpCollector::start()
            .await
            .expect("Failed to start OTLP collector");
        let otlp_endpoint = otlp_collector.http_metrics_endpoint();

        let non_existent_host = "192.0.2.1:9999";

        let router = TestRouter::builder()
            .inline_config(&format!(
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
                    all:
                        request_timeout: 500ms
                        circuit_breaker:
                            enabled: true
                            error_threshold: 50%
                            volume_threshold: 3
                            reset_timeout: 30s
                override_subgraph_urls:
                    subgraphs:
                        accounts:
                            url: "http://{non_existent_host}/accounts"
                "#
            ))
            .build()
            .start()
            .await;

        for _ in 1..=4 {
            router
                .send_graphql_request("{ users { id } }", None, None)
                .await;
        }

        for _ in 1..=3 {
            router
                .send_graphql_request("{ users { id } }", None, None)
                .await;
        }

        tokio::time::sleep(Duration::from_millis(100)).await;

        let metrics = otlp_collector.metrics_view().await;
        let attrs = [(labels::SUBGRAPH_NAME, "accounts")];

        let short_circuit_count =
            metrics.latest_counter(names::CIRCUIT_BREAKER_SHORT_CIRCUITS_TOTAL, &attrs);

        assert!(
            short_circuit_count >= 3.0,
            "Expected at least 3 short-circuited requests, got {}",
            short_circuit_count
        );
    }

    /// The `state` gauge must report `0` (closed) once a circuit breaker is
    /// configured for a subgraph, even before any traffic has hit it.
    #[ntex::test]
    async fn should_expose_closed_state_baseline_for_configured_subgraphs() {
        let otlp_collector = OtlpCollector::start()
            .await
            .expect("Failed to start OTLP collector");
        let otlp_endpoint = otlp_collector.http_metrics_endpoint();

        let router = TestRouter::builder()
            .inline_config(&format!(
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
                    all:
                        circuit_breaker:
                            enabled: true
                            error_threshold: 50%
                            volume_threshold: 3
                            reset_timeout: 30s
                "#
            ))
            .build()
            .start()
            .await;

        // Give the meter provider one export cycle so the observable gauge
        // can publish the initial baseline before we read it back.
        tokio::time::sleep(Duration::from_millis(150)).await;

        let metrics = otlp_collector.metrics_view().await;
        let attrs = [(labels::SUBGRAPH_NAME, "accounts")];
        let state = metrics.latest_gauge(names::CIRCUIT_BREAKER_STATE, &attrs);
        assert_eq!(
            state, 0.0,
            "state gauge must be 0 (closed) before any traffic, got {state}"
        );

        // Quiet "unused" warning in case `router` would otherwise be dropped
        // before the metric is collected.
        drop(router);
    }

    /// When the breaker trips, the `state` gauge must flip to `1` (open) and
    /// a single `closed -> open` transition must be recorded.
    #[ntex::test]
    async fn should_record_state_open_and_transition_metrics() {
        let otlp_collector = OtlpCollector::start()
            .await
            .expect("Failed to start OTLP collector");
        let otlp_endpoint = otlp_collector.http_metrics_endpoint();

        let non_existent_host = "192.0.2.1:9999";

        let router = TestRouter::builder()
            .inline_config(&format!(
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
                    all:
                        request_timeout: 500ms
                        circuit_breaker:
                            enabled: true
                            error_threshold: 50%
                            volume_threshold: 3
                            reset_timeout: 30s
                override_subgraph_urls:
                    subgraphs:
                        accounts:
                            url: "http://{non_existent_host}/accounts"
                "#
            ))
            .build()
            .start()
            .await;

        // Trip the breaker.
        for _ in 1..=5 {
            router
                .send_graphql_request("{ users { id } }", None, None)
                .await;
        }

        tokio::time::sleep(Duration::from_millis(150)).await;

        let metrics = otlp_collector.metrics_view().await;
        let attrs = [(labels::SUBGRAPH_NAME, "accounts")];

        let state = metrics.latest_gauge(names::CIRCUIT_BREAKER_STATE, &attrs);
        assert_eq!(state, 1.0, "state gauge must report 1 (open), got {state}");

        let transitions = metrics.latest_counter(
            names::CIRCUIT_BREAKER_STATE_TRANSITIONS_TOTAL,
            &[
                (labels::SUBGRAPH_NAME, "accounts"),
                (labels::CIRCUIT_BREAKER_FROM_STATE, "closed"),
                (labels::CIRCUIT_BREAKER_TO_STATE, "open"),
            ],
        );
        assert!(
            transitions >= 1.0,
            "Expected at least one closed->open transition, got {transitions}"
        );
    }

    /// The breaker must count subgraph errors that it permitted (i.e. the
    /// inner future returned `Err`) under `failures_total`, separately from
    /// short-circuited requests.
    #[ntex::test]
    async fn should_record_failure_metrics_for_permitted_errors() {
        let mut accounts_server = mockito::Server::new_async().await;
        let host = accounts_server.host_with_port();

        // Volume threshold is high enough that no request gets short-circuited
        // in this run, so every failing request must show up under failures.
        let error_mock = accounts_server
            .mock("POST", "/accounts")
            .with_status(503)
            .expect(3)
            .create_async()
            .await;

        let otlp_collector = OtlpCollector::start()
            .await
            .expect("Failed to start OTLP collector");
        let otlp_endpoint = otlp_collector.http_metrics_endpoint();

        let router = TestRouter::builder()
            .inline_config(&format!(
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
                    all:
                        circuit_breaker:
                            enabled: true
                            error_threshold: 99%
                            volume_threshold: 100
                            reset_timeout: 30s
                override_subgraph_urls:
                    subgraphs:
                        accounts:
                            url: "http://{host}/accounts"
                "#
            ))
            .build()
            .start()
            .await;

        for _ in 1..=3 {
            let _ = router
                .send_graphql_request("{ users { id } }", None, None)
                .await;
        }

        error_mock.assert_async().await;
        tokio::time::sleep(Duration::from_millis(150)).await;

        let metrics = otlp_collector.metrics_view().await;
        let attrs = [(labels::SUBGRAPH_NAME, "accounts")];

        let failures = metrics.latest_counter(names::CIRCUIT_BREAKER_FAILURES_TOTAL, &attrs);
        assert!(
            failures >= 3.0,
            "Expected at least 3 failures recorded, got {failures}"
        );

        let short_circuits =
            metrics.latest_counter(names::CIRCUIT_BREAKER_SHORT_CIRCUITS_TOTAL, &attrs);
        assert_eq!(
            short_circuits, 0.0,
            "Expected no short-circuits because volume_threshold was not reached, got {short_circuits}"
        );

        // The breaker never tripped, so state should still be `closed`.
        let state = metrics.latest_gauge(names::CIRCUIT_BREAKER_STATE, &attrs);
        assert_eq!(
            state, 0.0,
            "state gauge must remain 0 (closed), got {state}"
        );
    }

    /// Cancelling the client request (or any in-flight `RecloserFuture`) must
    /// not be misreported as a subgraph failure by the circuit breaker:
    /// `recloser`'s async future only calls `on_success` / `on_error` when
    /// the wrapped inner future resolves with `Poll::Ready`. Dropping the
    /// future mid-poll leaves the recloser state untouched.
    ///
    /// We exercise that contract by repeatedly issuing client requests to a
    /// subgraph that hangs forever and abandoning each request well before
    /// the router-level subgraph `request_timeout` fires (so no
    /// `SUBGRAPH_REQUEST_TIMEOUT` is produced either). The breaker thresholds
    /// are tight enough that even one mis-counted failure would open it.
    #[ntex::test]
    async fn should_not_trip_circuit_breaker_on_client_request_cancellation() {
        let otlp_collector = OtlpCollector::start()
            .await
            .expect("Failed to start OTLP collector");
        let otlp_endpoint = otlp_collector.http_metrics_endpoint();

        // Cooperative async sleep keeps the subgraph future `Pending` long
        // enough for every client request to be cancelled mid-flight.
        let subgraphs = TestSubgraphs::builder()
            .with_delay(Duration::from_secs(5))
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
                telemetry:
                    metrics:
                        exporters:
                            - kind: otlp
                              endpoint: {otlp_endpoint}
                              protocol: http
                              interval: 30ms
                              max_export_timeout: 50ms
                traffic_shaping:
                    all:
                        # Plenty of headroom so the router never times the
                        # subgraph out within the test window; only the
                        # client-side cancellation should affect the breaker.
                        request_timeout: 30s
                        circuit_breaker:
                            enabled: true
                            error_threshold: 50%
                            volume_threshold: 3
                            reset_timeout: 30s
                "#
            ))
            .build()
            .start()
            .await;

        // Five cancellations is well above `volume_threshold`; if drops were
        // ever counted as failures (e.g. > 50%), the breaker would open.
        for _ in 1..=5 {
            let _ = tokio::time::timeout(
                Duration::from_millis(100),
                router.send_graphql_request("{ users { id } }", None, None),
            )
            .await;
        }

        // Give the meter provider one export cycle.
        tokio::time::sleep(Duration::from_millis(200)).await;

        let metrics = otlp_collector.metrics_view().await;
        let attrs = [(labels::SUBGRAPH_NAME, "accounts")];

        let state = metrics.latest_gauge(names::CIRCUIT_BREAKER_STATE, &attrs);
        assert_eq!(
            state, 0.0,
            "state gauge must remain 0 (closed) after client cancellations, got {state}"
        );

        let short_circuits =
            metrics.latest_counter(names::CIRCUIT_BREAKER_SHORT_CIRCUITS_TOTAL, &attrs);
        assert_eq!(
            short_circuits, 0.0,
            "cancellations must not cause any short-circuited requests, got {short_circuits}"
        );

        let failures = metrics.latest_counter(names::CIRCUIT_BREAKER_FAILURES_TOTAL, &attrs);
        assert_eq!(
            failures, 0.0,
            "cancellations must not be counted as failures, got {failures}"
        );

        // The next request, once we let it run to completion, must still go
        // through, proving the breaker is genuinely closed and not just
        // closed-by-export-delay.
        let res = tokio::time::timeout(
            Duration::from_secs(7),
            router.send_graphql_request("{ users { id } }", None, None),
        )
        .await
        .expect("post-cancellation request must complete");
        assert_eq!(res.status(), ntex::http::StatusCode::OK);
        let body = res.json_body_string_pretty().await;
        assert!(
            !body.contains("SUBGRAPH_CIRCUIT_BREAKER_REJECTED"),
            "breaker must not reject after cancellations, got: {body}"
        );
    }

    /// When the circuit breaker rejects a request, the `SUBGRAPH_CIRCUIT_BREAKER_REJECTED`
    /// error must be recorded on both:
    /// - the intermediate `graphql.subgraph.operation` span, directly by the
    ///   executor's `prepare_execution_job` error hook, so the trace shows the
    ///   failure at the subgraph fetch level too, and
    /// - the top-level `graphql.operation` span, through the standard
    ///   response-error pipeline that aggregates every error attached to the
    ///   final GraphQL response.
    ///
    /// On each affected span we assert:
    /// - `hive.graphql.error.codes` attribute contains the code,
    /// - `hive.graphql.error.count` is at least 1,
    /// - a `graphql.error` event is attached with `error.type` set to the code
    ///   and `hive.error.subgraph_name` pointing at the rejected subgraph.
    #[ntex::test]
    async fn should_record_short_circuit_error_on_operation_span() {
        let otlp_collector = OtlpCollector::start()
            .await
            .expect("Failed to start OTLP collector");
        let otlp_endpoint = otlp_collector.http_traces_endpoint();

        let non_existent_host = "192.0.2.1:9999";

        let router = TestRouter::builder()
            .inline_config(&format!(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                telemetry:
                    tracing:
                        exporters:
                            - kind: otlp
                              endpoint: {otlp_endpoint}
                              protocol: http
                              batch_processor:
                                scheduled_delay: 50ms
                                max_export_timeout: 2s
                traffic_shaping:
                    all:
                        request_timeout: 200ms
                        circuit_breaker:
                            enabled: true
                            error_threshold: 50%
                            volume_threshold: 3
                            reset_timeout: 30s
                override_subgraph_urls:
                    subgraphs:
                        accounts:
                            url: "http://{non_existent_host}/accounts"
                "#
            ))
            .build()
            .start()
            .await;

        // Four timing-out requests are needed to actually trip the breaker:
        // the first three fill the volume_threshold=3 ring buffer (returning
        // a sentinel rate of -1.0), and the fourth one is the call whose
        // failure causes the recloser to evaluate the error rate and
        // transition from Closed to Open.
        for _ in 1..=4 {
            let _ = router
                .send_graphql_request("{ users { id } }", None, None)
                .await;
        }

        // This request should be short-circuited by the breaker.
        let res = router
            .send_graphql_request("{ users { id } }", None, None)
            .await;
        assert_eq!(res.status(), ntex::http::StatusCode::OK);
        let body = res.json_body_string_pretty().await;
        assert!(
            body.contains("SUBGRAPH_CIRCUIT_BREAKER_REJECTED"),
            "expected client response to carry the breaker rejection, got: {body}"
        );

        // Wait until traces for all 5 requests (4 timing-out + 1 rejected)
        // have been exported. Every request produces a
        // `graphql.subgraph.operation` span, including the short-circuited
        // one, so we use that as the readiness signal.
        let traces = otlp_collector
            .wait_for_traces_with_span(5, "graphql.subgraph.operation")
            .await;

        // The rejected request is the most recent one; pick the trace whose
        // `graphql.operation` span carries the circuit-breaker error code.
        let rejected_trace = traces
            .iter()
            .find(|trace| {
                trace.spans.iter().any(|span| {
                    span.attributes.get(HIVE_KIND).map(String::as_str) == Some("graphql.operation")
                        && span
                            .attributes
                            .get(HIVE_GRAPHQL_ERROR_CODES)
                            .map(|codes| codes.contains("SUBGRAPH_CIRCUIT_BREAKER_REJECTED"))
                            .unwrap_or(false)
                })
            })
            .expect("expected a trace whose graphql.operation span recorded the breaker rejection");

        let operation_span = rejected_trace.span_by_hive_kind_one("graphql.operation");
        assert_breaker_rejection_recorded_on_span(operation_span, "graphql.operation");

        let subgraph_operation_span =
            rejected_trace.span_by_hive_kind_one("graphql.subgraph.operation");
        assert_breaker_rejection_recorded_on_span(
            subgraph_operation_span,
            "graphql.subgraph.operation",
        );
    }

    /// When a subgraph defines a `circuit_breaker` block but does not explicitly
    /// set `enabled`, the value should be inherited from the global `all`
    /// configuration instead of silently defaulting to `false`.
    #[ntex::test]
    async fn should_inherit_enabled_from_global_when_subgraph_circuit_breaker_omits_enabled() {
        let mut accounts_server = mockito::Server::new_async().await;
        let host = accounts_server.host_with_port();

        let error_mock = accounts_server
            .mock("POST", "/accounts")
            .with_status(503)
            .expect(3)
            .create_async()
            .await;

        let router = TestRouter::builder()
            .inline_config(&format!(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                traffic_shaping:
                    all:
                        circuit_breaker:
                            enabled: true
                            error_threshold: 50%
                            volume_threshold: 2
                            reset_timeout: 30s
                    subgraphs:
                        accounts:
                            circuit_breaker:
                                volume_threshold: 2
                override_subgraph_urls:
                    subgraphs:
                        accounts:
                            url: "http://{host}/accounts"
                "#
            ))
            .build()
            .start()
            .await;

        for _ in 1..=3 {
            let _ = router
                .send_graphql_request("{ users { id } }", None, None)
                .await;
        }

        error_mock.assert_async().await;

        // The subgraph block exists without `enabled`, so it should inherit
        // `enabled: true` from the global config and trip the breaker.
        let res = router
            .send_graphql_request("{ users { id } }", None, None)
            .await;

        insta::assert_snapshot!(
            res.json_body_string_pretty().await,
            @r###"{
  "data": {
    "users": null
  },
  "errors": [
    {
      "message": "Rejected by the circuit breaker",
      "extensions": {
        "code": "SUBGRAPH_CIRCUIT_BREAKER_REJECTED",
        "serviceName": "accounts"
      }
    }
  ]
}"###
        );
    }

    /// A subgraph should be able to explicitly opt out of the circuit breaker
    /// (with `enabled: false`) even when the global config has it enabled.
    #[ntex::test]
    async fn should_allow_subgraph_to_disable_circuit_breaker_when_global_is_enabled() {
        let mut accounts_server = mockito::Server::new_async().await;
        let host = accounts_server.host_with_port();

        // The breaker should never trip for `accounts`, so all 10 requests must
        // reach the upstream subgraph.
        let error_mock = accounts_server
            .mock("POST", "/accounts")
            .with_status(503)
            .expect(10)
            .create_async()
            .await;

        let router = TestRouter::builder()
            .inline_config(&format!(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                traffic_shaping:
                    all:
                        circuit_breaker:
                            enabled: true
                            error_threshold: 50%
                            volume_threshold: 3
                            reset_timeout: 30s
                    subgraphs:
                        accounts:
                            circuit_breaker:
                                enabled: false
                override_subgraph_urls:
                    subgraphs:
                        accounts:
                            url: "http://{host}/accounts"
                "#
            ))
            .build()
            .start()
            .await;

        for _ in 1..=10 {
            let _ = router
                .send_graphql_request("{ users { id } }", None, None)
                .await;
        }

        error_mock.assert_async().await;

        let res = router
            .send_graphql_request("{ users { id } }", None, None)
            .await;

        // The breaker stays closed, so we keep getting the upstream error
        // instead of `SUBGRAPH_CIRCUIT_BREAKER_REJECTED`.
        insta::assert_snapshot!(
            res.json_body_string_pretty().await,
            @r###"{
  "data": {
    "users": null
  },
  "errors": [
    {
      "message": "Received empty response body from subgraph \"accounts\"",
      "extensions": {
        "code": "SUBGRAPH_RESPONSE_BODY_EMPTY",
        "serviceName": "accounts"
      }
    }
  ]
}"###
        );
    }

    /// When a subgraph responds with HTTP 503 but ships a valid GraphQL body
    /// (data and/or errors), the router must surface that body to the client
    /// instead of replacing it with an `SUBGRAPH_RESPONSE_BODY_EMPTY` error.
    /// The 503 is still recorded internally so the circuit breaker can trip,
    /// but a single failing request alone should not transform the upstream
    /// response.
    #[ntex::test]
    async fn should_return_subgraph_503_response_body_to_client() {
        let mut accounts_server = mockito::Server::new_async().await;
        let host = accounts_server.host_with_port();

        let error_mock = accounts_server
            .mock("POST", "/accounts")
            .with_status(503)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"data":{"users":[{"id":"1"}]},"errors":[{"message":"upstream is unhappy","extensions":{"code":"UPSTREAM_FAILURE"}}]}"#,
            )
            .expect_at_least(1)
            .create_async()
            .await;

        let router = TestRouter::builder()
            .inline_config(&format!(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                traffic_shaping:
                    all:
                        circuit_breaker:
                            enabled: false
                override_subgraph_urls:
                    subgraphs:
                        accounts:
                            url: "http://{host}/accounts"
                "#
            ))
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request("{ users { id } }", None, None)
            .await;
        assert_eq!(res.status(), ntex::http::StatusCode::OK);

        // The upstream 503 body must be propagated to the client (with the
        // partial `data` and upstream `errors`) instead of being replaced by
        // an `SUBGRAPH_RESPONSE_BODY_EMPTY` error. The router augments the
        // error extensions with the originating subgraph name.
        insta::assert_snapshot!(
            res.json_body_string_pretty().await,
            @r###"{
  "data": {
    "users": [
      {
        "id": "1"
      }
    ]
  },
  "errors": [
    {
      "message": "upstream is unhappy",
      "extensions": {
        "code": "UPSTREAM_FAILURE",
        "serviceName": "accounts"
      }
    }
  ]
}"###
        );

        error_mock.assert_async().await;
    }

    /// 503 subgraph responses that carry a valid GraphQL body must still count
    /// as failures for the circuit breaker (503 is among the default tracked
    /// status codes). The first few requests should receive the upstream body, then
    /// the breaker must open and short-circuit subsequent requests with
    /// `SUBGRAPH_CIRCUIT_BREAKER_REJECTED`.
    #[ntex::test]
    async fn should_track_503_responses_with_body_in_circuit_breaker() {
        let mut accounts_server = mockito::Server::new_async().await;
        let host = accounts_server.host_with_port();

        // The breaker should open after 4 failures with these thresholds, so
        // the upstream subgraph should be hit exactly 4 times.
        let error_mock = accounts_server
            .mock("POST", "/accounts")
            .with_status(503)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"data":null,"errors":[{"message":"upstream is unhappy","extensions":{"code":"UPSTREAM_FAILURE"}}]}"#,
            )
            .expect(4)
            .create_async()
            .await;

        let router = TestRouter::builder()
            .inline_config(&format!(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                traffic_shaping:
                    all:
                        circuit_breaker:
                            enabled: true
                            error_threshold: 50%
                            volume_threshold: 3
                            reset_timeout: 30s
                override_subgraph_urls:
                    subgraphs:
                        accounts:
                            url: "http://{host}/accounts"
                "#
            ))
            .build()
            .start()
            .await;

        // First four requests reach the subgraph and surface the upstream
        // 503 body to the client. We collect the bodies first so the snapshot
        // assertion can live outside the loop (insta forbids inline snapshots
        // inside loops).
        let mut bodies = Vec::with_capacity(4);
        for _ in 1..=4 {
            let res = router
                .send_graphql_request("{ users { id } }", None, None)
                .await;
            assert_eq!(res.status(), ntex::http::StatusCode::OK);
            bodies.push(res.json_body_string_pretty().await);
        }
        // All four bodies should be identical upstream 503 payloads.
        for body in &bodies[1..] {
            assert_eq!(&bodies[0], body);
        }
        insta::assert_snapshot!(
            bodies[0],
            @r###"{
  "data": {
    "users": null
  },
  "errors": [
    {
      "message": "upstream is unhappy",
      "extensions": {
        "code": "UPSTREAM_FAILURE",
        "serviceName": "accounts"
      }
    }
  ]
}"###
        );

        error_mock.assert_async().await;

        // After the threshold is reached, the breaker opens and rejects new
        // requests without ever calling the subgraph.
        let res = router
            .send_graphql_request("{ users { id } }", None, None)
            .await;
        assert_eq!(res.status(), ntex::http::StatusCode::OK);
        insta::assert_snapshot!(
            res.json_body_string_pretty().await,
            @r###"{
  "data": {
    "users": null
  },
  "errors": [
    {
      "message": "Rejected by the circuit breaker",
      "extensions": {
        "code": "SUBGRAPH_CIRCUIT_BREAKER_REJECTED",
        "serviceName": "accounts"
      }
    }
  ]
}"###
        );
    }

    /// By default the circuit breaker only tracks the canonical infrastructure
    /// 5xx codes (`500`, `502`, `503`, `504`). Other 5xx statuses (e.g. `505`)
    /// must NOT count as failures, so the subgraph keeps being called for
    /// every request.
    #[ntex::test]
    async fn should_not_open_circuit_breaker_for_status_outside_default_set() {
        let mut accounts_server = mockito::Server::new_async().await;
        let host = accounts_server.host_with_port();

        // Subgraph returns a valid GraphQL body alongside the 505 status,
        // so the executor surfaces it as a successful response (with the
        // upstream status). Without a body the executor would return an
        // error which the breaker counts regardless of status code.
        let error_mock = accounts_server
            .mock("POST", "/accounts")
            .with_status(505)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data":null,"errors":[{"message":"oops"}]}"#)
            .expect(6)
            .create_async()
            .await;

        let router = TestRouter::builder()
            .inline_config(&format!(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                traffic_shaping:
                    all:
                        circuit_breaker:
                            enabled: true
                            error_threshold: 50%
                            volume_threshold: 3
                            reset_timeout: 30s
                override_subgraph_urls:
                    subgraphs:
                        accounts:
                            url: "http://{host}/accounts"
                "#
            ))
            .build()
            .start()
            .await;

        for _ in 1..=6 {
            let res = router
                .send_graphql_request("{ users { id } }", None, None)
                .await;
            // The breaker never opens, so the upstream body is always
            // returned and we never see SUBGRAPH_CIRCUIT_BREAKER_REJECTED.
            let body = res.json_body_string_pretty().await;
            assert!(
                !body.contains("SUBGRAPH_CIRCUIT_BREAKER_REJECTED"),
                "breaker must not open for status codes outside the default set, got: {body}"
            );
        }

        error_mock.assert_async().await;
    }

    /// When the user configures `error_status_codes` to include 500, the
    /// circuit breaker must open for 500 responses just like it does for 503
    /// by default.
    #[ntex::test]
    async fn should_open_circuit_breaker_on_500_when_configured() {
        let mut accounts_server = mockito::Server::new_async().await;
        let host = accounts_server.host_with_port();

        // The breaker should open after 4 failures with these thresholds, so
        // the subgraph should be hit exactly 4 times.
        let error_mock = accounts_server
            .mock("POST", "/accounts")
            .with_status(500)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data":null,"errors":[{"message":"oops"}]}"#)
            .expect(4)
            .create_async()
            .await;

        let router = TestRouter::builder()
            .inline_config(&format!(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                traffic_shaping:
                    all:
                        circuit_breaker:
                            enabled: true
                            error_threshold: 50%
                            volume_threshold: 3
                            reset_timeout: 30s
                            error_status_codes: [500, 502, 503, 504]
                override_subgraph_urls:
                    subgraphs:
                        accounts:
                            url: "http://{host}/accounts"
                "#
            ))
            .build()
            .start()
            .await;

        for _ in 1..=4 {
            let _ = router
                .send_graphql_request("{ users { id } }", None, None)
                .await;
        }

        let res = router
            .send_graphql_request("{ users { id } }", None, None)
            .await;
        assert_eq!(res.status(), ntex::http::StatusCode::OK);
        insta::assert_snapshot!(
            res.json_body_string_pretty().await,
            @r###"{
  "data": {
    "users": null
  },
  "errors": [
    {
      "message": "Rejected by the circuit breaker",
      "extensions": {
        "code": "SUBGRAPH_CIRCUIT_BREAKER_REJECTED",
        "serviceName": "accounts"
      }
    }
  ]
}"###
        );

        error_mock.assert_async().await;
    }

    /// A `"5xx"` wildcard entry in `error_status_codes` must treat every
    /// `500..=599` response as a failure, without requiring users to
    /// enumerate every status code by hand.
    #[ntex::test]
    async fn should_trip_circuit_breaker_on_5xx_wildcard_status_code() {
        let mut accounts_server = mockito::Server::new_async().await;
        let host = accounts_server.host_with_port();

        // 599 is outside the default-tracked set, so this trip can only
        // happen because the `"5xx"` wildcard expanded to cover it.
        let error_mock = accounts_server
            .mock("POST", "/accounts")
            .with_status(599)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data":null,"errors":[{"message":"server error"}]}"#)
            .expect(4)
            .create_async()
            .await;

        let router = TestRouter::builder()
            .inline_config(&format!(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                traffic_shaping:
                    all:
                        circuit_breaker:
                            enabled: true
                            error_threshold: 50%
                            volume_threshold: 3
                            reset_timeout: 30s
                            error_status_codes: ["5xx"]
                override_subgraph_urls:
                    subgraphs:
                        accounts:
                            url: "http://{host}/accounts"
                "#
            ))
            .build()
            .start()
            .await;

        for _ in 1..=4 {
            let _ = router
                .send_graphql_request("{ users { id } }", None, None)
                .await;
        }

        let res = router
            .send_graphql_request("{ users { id } }", None, None)
            .await;
        let body = res.json_body_string_pretty().await;
        assert!(
            body.contains("SUBGRAPH_CIRCUIT_BREAKER_REJECTED"),
            "expected the breaker to trip after 4x 599 with [\"5xx\"], got: {body}"
        );

        error_mock.assert_async().await;
    }

    /// A `"50x"` wildcard entry must cover the `500..=509` range. 504 is
    /// inside that range, so 4 failing responses must be enough to trip
    /// the breaker.
    #[ntex::test]
    async fn should_trip_circuit_breaker_on_50x_wildcard_status_code() {
        let mut accounts_server = mockito::Server::new_async().await;
        let host = accounts_server.host_with_port();

        let error_mock = accounts_server
            .mock("POST", "/accounts")
            .with_status(504)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data":null,"errors":[{"message":"gateway timeout"}]}"#)
            .expect(4)
            .create_async()
            .await;

        let router = TestRouter::builder()
            .inline_config(&format!(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                traffic_shaping:
                    all:
                        circuit_breaker:
                            enabled: true
                            error_threshold: 50%
                            volume_threshold: 3
                            reset_timeout: 30s
                            error_status_codes: ["50x"]
                override_subgraph_urls:
                    subgraphs:
                        accounts:
                            url: "http://{host}/accounts"
                "#
            ))
            .build()
            .start()
            .await;

        for _ in 1..=4 {
            let _ = router
                .send_graphql_request("{ users { id } }", None, None)
                .await;
        }

        let res = router
            .send_graphql_request("{ users { id } }", None, None)
            .await;
        let body = res.json_body_string_pretty().await;
        assert!(
            body.contains("SUBGRAPH_CIRCUIT_BREAKER_REJECTED"),
            "expected the breaker to trip after 4x 504 with [\"50x\"], got: {body}"
        );

        error_mock.assert_async().await;
    }

    /// Status codes that fall outside the wildcard's range must not be
    /// counted as failures. 510 is outside `"50x"` (500..=509), so 6
    /// failing responses in a row must keep the breaker closed.
    #[ntex::test]
    async fn should_not_trip_circuit_breaker_for_status_code_outside_wildcard_range() {
        let mut accounts_server = mockito::Server::new_async().await;
        let host = accounts_server.host_with_port();

        let error_mock = accounts_server
            .mock("POST", "/accounts")
            .with_status(510)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data":null,"errors":[{"message":"not extended"}]}"#)
            .expect(6)
            .create_async()
            .await;

        let router = TestRouter::builder()
            .inline_config(&format!(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                traffic_shaping:
                    all:
                        circuit_breaker:
                            enabled: true
                            error_threshold: 50%
                            volume_threshold: 3
                            reset_timeout: 30s
                            error_status_codes: ["50x"]
                override_subgraph_urls:
                    subgraphs:
                        accounts:
                            url: "http://{host}/accounts"
                "#
            ))
            .build()
            .start()
            .await;

        for _ in 1..=6 {
            let res = router
                .send_graphql_request("{ users { id } }", None, None)
                .await;
            let body = res.json_body_string_pretty().await;
            assert!(
                !body.contains("SUBGRAPH_CIRCUIT_BREAKER_REJECTED"),
                "510 must not match the \"50x\" wildcard, so the breaker must stay closed, got: {body}"
            );
        }

        error_mock.assert_async().await;
    }

    /// Integer codes and wildcard patterns must be acceptable side by side
    /// in the same `error_status_codes` list. With `[501, "52x"]`, 520 is
    /// only matched by the wildcard, so seeing the breaker trip on 520
    /// proves that wildcards still apply when mixed with explicit codes.
    #[ntex::test]
    async fn should_accept_mixed_integer_and_wildcard_status_codes() {
        let mut accounts_server = mockito::Server::new_async().await;
        let host = accounts_server.host_with_port();

        let error_mock = accounts_server
            .mock("POST", "/accounts")
            .with_status(520)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data":null,"errors":[{"message":"cloudflare unknown"}]}"#)
            .expect(4)
            .create_async()
            .await;

        let router = TestRouter::builder()
            .inline_config(&format!(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                traffic_shaping:
                    all:
                        circuit_breaker:
                            enabled: true
                            error_threshold: 50%
                            volume_threshold: 3
                            reset_timeout: 30s
                            error_status_codes: [501, "52x"]
                override_subgraph_urls:
                    subgraphs:
                        accounts:
                            url: "http://{host}/accounts"
                "#
            ))
            .build()
            .start()
            .await;

        for _ in 1..=4 {
            let _ = router
                .send_graphql_request("{ users { id } }", None, None)
                .await;
        }

        let res = router
            .send_graphql_request("{ users { id } }", None, None)
            .await;
        let body = res.json_body_string_pretty().await;
        assert!(
            body.contains("SUBGRAPH_CIRCUIT_BREAKER_REJECTED"),
            "expected the breaker to trip after 4x 520 with [501, \"52x\"], got: {body}"
        );

        error_mock.assert_async().await;
    }

    /// A subgraph-level `error_status_codes` setting must override the
    /// global one for that specific subgraph.
    #[ntex::test]
    async fn should_use_subgraph_level_error_status_codes_override() {
        let mut accounts_server = mockito::Server::new_async().await;
        let host = accounts_server.host_with_port();

        // Subgraph-level config restricts failures to 502 only, so even
        // though the global config would treat 500 as a failure, 500
        // responses must not trip the breaker for `accounts`.
        let error_mock = accounts_server
            .mock("POST", "/accounts")
            .with_status(500)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data":null,"errors":[{"message":"oops"}]}"#)
            .expect(6)
            .create_async()
            .await;

        let router = TestRouter::builder()
            .inline_config(&format!(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                traffic_shaping:
                    all:
                        circuit_breaker:
                            enabled: true
                            error_threshold: 50%
                            volume_threshold: 3
                            reset_timeout: 30s
                            error_status_codes: [500]
                    subgraphs:
                        accounts:
                            circuit_breaker:
                                error_status_codes: [502]
                override_subgraph_urls:
                    subgraphs:
                        accounts:
                            url: "http://{host}/accounts"
                "#
            ))
            .build()
            .start()
            .await;

        for _ in 1..=6 {
            let res = router
                .send_graphql_request("{ users { id } }", None, None)
                .await;
            let body = res.json_body_string_pretty().await;
            assert!(
                !body.contains("SUBGRAPH_CIRCUIT_BREAKER_REJECTED"),
                "subgraph-level override must prevent breaker from opening on 500, got: {body}"
            );
        }

        error_mock.assert_async().await;
    }

    /// When the subgraph that backs a subscription fails to accept the initial
    /// HTTP request (i.e. the establishment of the subscription returns an
    /// error), that failure must be counted by the circuit breaker exactly
    /// like a regular query failure. With a high `volume_threshold` the
    /// breaker should not open yet, so we only assert on `failures_total`.
    #[ntex::test]
    async fn should_record_failure_metric_on_subscription_establishment_error() {
        let subgraphs = TestSubgraphs::builder()
            .with_on_request(|req| {
                if req.path == "/reviews" {
                    Some(ResponseLike::new(
                        reqwest::StatusCode::INTERNAL_SERVER_ERROR,
                        None,
                        None,
                    ))
                } else {
                    None
                }
            })
            .build()
            .start()
            .await;

        let otlp_collector = OtlpCollector::start()
            .await
            .expect("Failed to start OTLP collector");
        let otlp_endpoint = otlp_collector.http_metrics_endpoint();

        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(&format!(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                subscriptions:
                    enabled: true
                telemetry:
                    metrics:
                        exporters:
                            - kind: otlp
                              endpoint: {otlp_endpoint}
                              protocol: http
                              interval: 30ms
                              max_export_timeout: 50ms
                traffic_shaping:
                    all:
                        circuit_breaker:
                            enabled: true
                            error_threshold: 99%
                            volume_threshold: 100
                            reset_timeout: 30s
                "#
            ))
            .build()
            .start()
            .await;

        for _ in 1..=3 {
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
                        ntex::http::header::ACCEPT => "text/event-stream"
                    },
                )
                .await;
            assert_eq!(res.status(), ntex::http::StatusCode::OK);
            let body = res.body().await.unwrap();
            let body_str = std::str::from_utf8(&body).unwrap();
            assert!(
                body_str.contains("SUBGRAPH_STREAM_STATUS_CODE_NOT_OK"),
                "expected the subscription to surface the upstream 500 as a stream error, got: {body_str}"
            );
        }

        tokio::time::sleep(Duration::from_millis(150)).await;

        let metrics = otlp_collector.metrics_view().await;
        let attrs = [(labels::SUBGRAPH_NAME, "reviews")];

        let failures = metrics.latest_counter(names::CIRCUIT_BREAKER_FAILURES_TOTAL, &attrs);
        assert!(
            failures >= 3.0,
            "Expected at least 3 subscription-establishment failures, got {failures}"
        );

        let short_circuits =
            metrics.latest_counter(names::CIRCUIT_BREAKER_SHORT_CIRCUITS_TOTAL, &attrs);
        assert_eq!(
            short_circuits, 0.0,
            "Expected no short-circuits because volume_threshold was not reached, got {short_circuits}"
        );

        let state = metrics.latest_gauge(names::CIRCUIT_BREAKER_STATE, &attrs);
        assert_eq!(
            state, 0.0,
            "state gauge must remain 0 (closed) below volume_threshold, got {state}"
        );
    }

    /// Once the subscription subgraph has failed enough times during
    /// establishment, the circuit breaker must trip and short-circuit the
    /// next subscription request before it ever reaches the subgraph, just
    /// like for regular queries. The error must be surfaced to the client
    /// through the SSE stream with `SUBGRAPH_CIRCUIT_BREAKER_REJECTED`.
    #[ntex::test]
    async fn should_short_circuit_subscription_when_breaker_is_open() {
        let subgraphs = TestSubgraphs::builder()
            .with_on_request(|req| {
                if req.path == "/reviews" {
                    Some(ResponseLike::new(
                        reqwest::StatusCode::INTERNAL_SERVER_ERROR,
                        None,
                        None,
                    ))
                } else {
                    None
                }
            })
            .build()
            .start()
            .await;

        let otlp_collector = OtlpCollector::start()
            .await
            .expect("Failed to start OTLP collector");
        let otlp_endpoint = otlp_collector.http_metrics_endpoint();

        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(&format!(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                subscriptions:
                    enabled: true
                telemetry:
                    metrics:
                        exporters:
                            - kind: otlp
                              endpoint: {otlp_endpoint}
                              protocol: http
                              interval: 30ms
                              max_export_timeout: 50ms
                traffic_shaping:
                    all:
                        circuit_breaker:
                            enabled: true
                            error_threshold: 50%
                            volume_threshold: 3
                            reset_timeout: 30s
                "#
            ))
            .build()
            .start()
            .await;

        // Send enough failing establishment attempts to trip the breaker.
        // `volume_threshold: 3` requires 4 errors before the breaker can open,
        // so the 5th attempt is the first one that is short-circuited.
        for _ in 1..=4 {
            let _ = router
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
                        ntex::http::header::ACCEPT => "text/event-stream"
                    },
                )
                .await;
        }

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
                    ntex::http::header::ACCEPT => "text/event-stream"
                },
            )
            .await;
        assert_eq!(res.status(), ntex::http::StatusCode::OK);
        let body = res.body().await.unwrap();
        let body_str = std::str::from_utf8(&body).unwrap();
        assert!(
            body_str.contains("SUBGRAPH_CIRCUIT_BREAKER_REJECTED"),
            "expected the rejected subscription to carry SUBGRAPH_CIRCUIT_BREAKER_REJECTED in its SSE body, got: {body_str}"
        );

        tokio::time::sleep(Duration::from_millis(150)).await;

        let metrics = otlp_collector.metrics_view().await;
        let attrs = [(labels::SUBGRAPH_NAME, "reviews")];

        let state = metrics.latest_gauge(names::CIRCUIT_BREAKER_STATE, &attrs);
        assert_eq!(state, 1.0, "state gauge must report 1 (open), got {state}");

        let short_circuits =
            metrics.latest_counter(names::CIRCUIT_BREAKER_SHORT_CIRCUITS_TOTAL, &attrs);
        assert!(
            short_circuits >= 1.0,
            "Expected at least one short-circuited subscription, got {short_circuits}"
        );

        let transitions = metrics.latest_counter(
            names::CIRCUIT_BREAKER_STATE_TRANSITIONS_TOTAL,
            &[
                (labels::SUBGRAPH_NAME, "reviews"),
                (labels::CIRCUIT_BREAKER_FROM_STATE, "closed"),
                (labels::CIRCUIT_BREAKER_TO_STATE, "open"),
            ],
        );
        assert!(
            transitions >= 1.0,
            "Expected at least one closed->open transition for reviews, got {transitions}"
        );
    }

    /// Per the agreed semantics, the circuit breaker only guards subscription
    /// _establishment_: errors that surface after the subscription has been
    /// successfully opened (downstream entity resolution failures, mid-stream
    /// GraphQL errors etc.) must not be counted against the originating
    /// subgraph's breaker. Once the subgraph has accepted the subscription
    /// we already know it is reachable; tearing the breaker open over
    /// post-establishment errors would conflate transport health with
    /// subgraph-level resolver issues.
    #[ntex::test]
    async fn should_not_trip_circuit_breaker_on_post_establishment_subscription_errors() {
        // `reviews` accepts subscriptions normally; `products` (used for
        // entity resolution after each event) always fails. This forces
        // every streamed event to carry a downstream error while keeping
        // the subscription itself successfully established.
        let subgraphs = TestSubgraphs::builder()
            .with_on_request(|req| {
                if req.path == "/products" {
                    Some(ResponseLike::new(
                        reqwest::StatusCode::INTERNAL_SERVER_ERROR,
                        Some(
                            sonic_rs::json!({
                                "errors": [{"message": "products is down"}]
                            })
                            .to_string(),
                        ),
                        some_header_map! {
                            ntex::http::header::CONTENT_TYPE => "application/json"
                        },
                    ))
                } else {
                    None
                }
            })
            .build()
            .start()
            .await;

        let otlp_collector = OtlpCollector::start()
            .await
            .expect("Failed to start OTLP collector");
        let otlp_endpoint = otlp_collector.http_metrics_endpoint();

        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(&format!(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                subscriptions:
                    enabled: true
                telemetry:
                    metrics:
                        exporters:
                            - kind: otlp
                              endpoint: {otlp_endpoint}
                              protocol: http
                              interval: 30ms
                              max_export_timeout: 50ms
                traffic_shaping:
                    all:
                        circuit_breaker:
                            enabled: true
                            error_threshold: 50%
                            volume_threshold: 3
                            reset_timeout: 30s
                    subgraphs:
                        # We rely on the products subgraph keeping on serving
                        # 500s so the entity-resolution errors surface in the
                        # subscription stream. Disabling the breaker for it
                        # isolates the test to the `reviews` breaker, which
                        # is what we actually care about here.
                        products:
                            circuit_breaker:
                                enabled: false
                "#
            ))
            .build()
            .start()
            .await;

        // Run a few subscriptions whose establishment succeeds but whose
        // entity-resolution fetches all fail. The breaker on `reviews`
        // must stay closed and report no failures.
        for _ in 1..=3 {
            let res = router
                .send_graphql_request(
                    r#"
                    subscription ($upc: String!) {
                        reviewAddedForProduct(productUpc: $upc, intervalInMs: 0) {
                            product {
                                name
                            }
                        }
                    }
                    "#,
                    Some(sonic_rs::json!({ "upc": "2" })),
                    some_header_map! {
                        ntex::http::header::ACCEPT => "text/event-stream"
                    },
                )
                .await;
            assert_eq!(res.status(), ntex::http::StatusCode::OK);
            let body = res.body().await.unwrap();
            let body_str = std::str::from_utf8(&body).unwrap();
            assert!(
                body_str.contains("DOWNSTREAM_SERVICE_ERROR"),
                "expected the stream to carry downstream entity-resolution errors, got: {body_str}"
            );
            assert!(
                !body_str.contains("SUBGRAPH_CIRCUIT_BREAKER_REJECTED"),
                "subscription establishment succeeded, breaker must not reject the request, got: {body_str}"
            );
        }

        tokio::time::sleep(Duration::from_millis(150)).await;

        let metrics = otlp_collector.metrics_view().await;
        let reviews_attrs = [(labels::SUBGRAPH_NAME, "reviews")];

        let failures =
            metrics.latest_counter(names::CIRCUIT_BREAKER_FAILURES_TOTAL, &reviews_attrs);
        assert_eq!(
            failures, 0.0,
            "post-establishment stream errors must not count against the reviews breaker, got {failures}"
        );

        let short_circuits =
            metrics.latest_counter(names::CIRCUIT_BREAKER_SHORT_CIRCUITS_TOTAL, &reviews_attrs);
        assert_eq!(
            short_circuits, 0.0,
            "breaker must not have rejected any subscriptions, got {short_circuits}"
        );

        let state = metrics.latest_gauge(names::CIRCUIT_BREAKER_STATE, &reviews_attrs);
        assert_eq!(
            state, 0.0,
            "reviews breaker must remain closed after post-establishment errors, got {state}"
        );
    }

    /// `half_open_attempts` controls how many probe requests fill the
    /// breaker's rolling sample after `reset_timeout` elapses. The probe
    /// that follows the filled sample is the one whose result decides
    /// whether to transition back to `Closed` or `Open`. With
    /// `half_open_attempts: 2` three probes pass through; if all three
    /// fail the breaker must transition back to `Open` and reject the
    /// very next request.
    #[ntex::test]
    async fn should_reopen_circuit_breaker_when_half_open_probes_fail() {
        let mut accounts_server = mockito::Server::new_async().await;
        let host = accounts_server.host_with_port();

        // 4 requests are required to trip the breaker (volume_threshold: 3,
        // 4th call rolls the buffer past the threshold), then 3 probe
        // requests during half-open (half_open_attempts: 2 fills the
        // sample, the 3rd probe triggers the transition). We expect
        // exactly 7 calls.
        let error_mock = accounts_server
            .mock("POST", "/accounts")
            .with_status(503)
            .expect(7)
            .create_async()
            .await;

        let router = TestRouter::builder()
            .inline_config(&format!(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                traffic_shaping:
                    all:
                        circuit_breaker:
                            enabled: true
                            error_threshold: 50%
                            volume_threshold: 3
                            reset_timeout: 1s
                            half_open_attempts: 2
                override_subgraph_urls:
                    subgraphs:
                        accounts:
                            url: "http://{host}/accounts"
                "#
            ))
            .build()
            .start()
            .await;

        for _ in 1..=4 {
            let _ = router
                .send_graphql_request("{ users { id } }", None, None)
                .await;
        }

        let rejected = router
            .send_graphql_request("{ users { id } }", None, None)
            .await;
        assert!(
            rejected
                .json_body_string_pretty()
                .await
                .contains("SUBGRAPH_CIRCUIT_BREAKER_REJECTED"),
            "breaker must be open after volume_threshold failing requests"
        );

        tokio::time::sleep(Duration::from_millis(1200)).await;

        for _ in 1..=3 {
            let res = router
                .send_graphql_request("{ users { id } }", None, None)
                .await;
            let body = res.json_body_string_pretty().await;
            assert!(
                !body.contains("SUBGRAPH_CIRCUIT_BREAKER_REJECTED"),
                "half-open probes must reach the failing subgraph, got: {body}"
            );
        }

        let reopened = router
            .send_graphql_request("{ users { id } }", None, None)
            .await;
        let body = reopened.json_body_string_pretty().await;
        assert!(
            body.contains("SUBGRAPH_CIRCUIT_BREAKER_REJECTED"),
            "after the half-open sample fills with failures the breaker must transition back to open, got: {body}"
        );

        error_mock.assert_async().await;
    }

    /// Mirror test: with `half_open_attempts: 2` three successful probes
    /// (two filling the sample plus a third that triggers evaluation)
    /// must transition the breaker from `HalfOpen` back to `Closed`,
    /// restoring normal traffic to the recovered subgraph.
    #[ntex::test]
    async fn should_close_circuit_breaker_when_half_open_probes_succeed() {
        let mut accounts_server = mockito::Server::new_async().await;
        let host = accounts_server.host_with_port();

        let error_mock = accounts_server
            .mock("POST", "/accounts")
            .with_status(503)
            .expect(4)
            .create_async()
            .await;

        let router = TestRouter::builder()
            .inline_config(&format!(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                traffic_shaping:
                    all:
                        circuit_breaker:
                            enabled: true
                            error_threshold: 50%
                            volume_threshold: 3
                            reset_timeout: 1s
                            half_open_attempts: 2
                override_subgraph_urls:
                    subgraphs:
                        accounts:
                            url: "http://{host}/accounts"
                "#
            ))
            .build()
            .start()
            .await;

        for _ in 1..=4 {
            let _ = router
                .send_graphql_request("{ users { id } }", None, None)
                .await;
        }

        let rejected = router
            .send_graphql_request("{ users { id } }", None, None)
            .await;
        assert!(
            rejected
                .json_body_string_pretty()
                .await
                .contains("SUBGRAPH_CIRCUIT_BREAKER_REJECTED"),
            "breaker must be open after volume_threshold failing requests"
        );

        error_mock.assert_async().await;

        let success_mock = accounts_server
            .mock("POST", "/accounts")
            .with_status(200)
            .with_body(r#"{"data":{"users":[{"id":"1"}]}}"#)
            .expect_at_least(4)
            .create_async()
            .await;

        tokio::time::sleep(Duration::from_millis(1200)).await;

        for _ in 1..=3 {
            let res = router
                .send_graphql_request("{ users { id } }", None, None)
                .await;
            let body = res.json_body_string_pretty().await;
            assert!(
                !body.contains("SUBGRAPH_CIRCUIT_BREAKER_REJECTED"),
                "half-open probes must reach the recovered subgraph, got: {body}"
            );
        }

        let closed = router
            .send_graphql_request("{ users { id } }", None, None)
            .await;
        insta::assert_snapshot!(
            closed.json_body_string_pretty().await,
            @r###"{
  "data": {
    "users": [
      {
        "id": "1"
      }
    ]
  }
}"###
        );

        success_mock.assert_async().await;
    }
}
