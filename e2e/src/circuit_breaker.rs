#[cfg(test)]
mod circuit_breaker_e2e_tests {
    use std::{thread::sleep, time::Duration};

    use hive_router_internal::telemetry::metrics::catalog::{labels, names};

    use crate::testkit::{otel::OtlpCollector, ClientResponseExt, TestRouter, TestSubgraphs};

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
            .with_status(500)
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
            .with_status(500)
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
            .with_status(500)
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

    #[ntex::test]
    async fn should_handle_circuit_breaker_with_mixed_success_and_failure() {
        let mut accounts_server = mockito::Server::new_async().await;
        let host = accounts_server.host_with_port();

        let mock = accounts_server
            .mock("POST", "/accounts")
            .with_status(500)
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
            .with_status(500)
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
            .with_status(500)
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

    #[ntex::test]
    async fn should_open_circuit_breaker_on_connection_errors() {
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
    async fn should_record_rejection_metrics() {
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

        let rejection_count =
            metrics.latest_counter(names::CIRCUIT_BREAKER_REJECTED_REQUESTS, &attrs);

        assert!(
            rejection_count >= 3.0,
            "Expected at least 3 rejected requests, got {}",
            rejection_count
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
            .with_status(500)
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
            .with_status(500)
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

    /// When a subgraph responds with HTTP 5xx but ships a valid GraphQL body
    /// (data and/or errors), the router must surface that body to the client
    /// instead of replacing it with an `SUBGRAPH_RESPONSE_BODY_EMPTY` error.
    /// The 5xx is still recorded internally so the circuit breaker can trip,
    /// but a single failing request alone should not transform the upstream
    /// response.
    #[ntex::test]
    async fn should_return_subgraph_5xx_response_body_to_client() {
        let mut accounts_server = mockito::Server::new_async().await;
        let host = accounts_server.host_with_port();

        let error_mock = accounts_server
            .mock("POST", "/accounts")
            .with_status(500)
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

        // The upstream 5xx body must be propagated to the client (with the
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

    /// 5xx subgraph responses that carry a valid GraphQL body must still count
    /// as failures for the circuit breaker. The first few requests should
    /// receive the upstream body, then the breaker must open and short-circuit
    /// subsequent requests with `SUBGRAPH_CIRCUIT_BREAKER_REJECTED`.
    #[ntex::test]
    async fn should_track_5xx_responses_with_body_in_circuit_breaker() {
        let mut accounts_server = mockito::Server::new_async().await;
        let host = accounts_server.host_with_port();

        // The breaker should open after 4 failures with these thresholds, so
        // the upstream subgraph should be hit exactly 4 times.
        let error_mock = accounts_server
            .mock("POST", "/accounts")
            .with_status(500)
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
                    accounts:
                        url: "http://{host}/accounts"
                "#
            ))
            .build()
            .start()
            .await;

        // First four requests reach the subgraph and surface the upstream
        // 5xx body to the client. We collect the bodies first so the snapshot
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
        // All four bodies should be identical upstream 5xx payloads.
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
}
