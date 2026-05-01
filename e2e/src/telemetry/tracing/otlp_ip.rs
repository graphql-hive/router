use crate::testkit::{otel::OtlpCollector, some_header_map, TestRouter, TestSubgraphs};

#[ntex::test]
async fn test_client_ip_default_uses_peer_and_ignores_xff() {
    let otlp_collector = OtlpCollector::start()
        .await
        .expect("Failed to start OTLP collector");
    let otlp_endpoint = otlp_collector.http_traces_endpoint();

    let subgraphs = TestSubgraphs::builder().build().start().await;
    let router = TestRouter::builder()
        .inline_config(format!(
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
      "#,
        ))
        .with_subgraphs(&subgraphs)
        .build()
        .start()
        .await;

    let response = router
        .send_graphql_request(
            "{ users { id } }",
            None,
            // These values should be ignored as ip_header is null
            some_header_map!(
              "x-forwarded-for" => "198.51.100.7, 10.0.0.2".to_string(),
              "x-real-ip" => "198.51.100.7",
              "forwarded" => "for=198.51.100.7"
            ),
        )
        .await;
    assert!(response.status().is_success());

    let http_server_span = otlp_collector
        .wait_for_span_by_hive_kind_one("http.server")
        .await;

    // Hive Router should use the peer address/port as the client address/port
    // regardless of the x-forwarded-for header as it's not defined.
    assert_eq!(
        http_server_span.attributes.get("client.address"),
        http_server_span.attributes.get("network.peer.address")
    );
    assert_eq!(
        http_server_span.attributes.get("client.port"),
        http_server_span.attributes.get("network.peer.port")
    );
}

#[ntex::test]
async fn test_client_ip_header_name_uses_left_most_value() {
    let otlp_collector = OtlpCollector::start()
        .await
        .expect("Failed to start OTLP collector");
    let otlp_endpoint = otlp_collector.http_traces_endpoint();

    let subgraphs = TestSubgraphs::builder().build().start().await;
    let router = TestRouter::builder()
        .inline_config(format!(
            r#"
          supergraph:
            source: file
            path: supergraph.graphql

          telemetry:
            client_identification:
              ip_header: x-forwarded-for
            tracing:
              exporters:
                - kind: otlp
                  endpoint: {otlp_endpoint}
                  protocol: http
                  batch_processor:
                    scheduled_delay: 50ms
                    max_export_timeout: 2s
      "#,
        ))
        .with_subgraphs(&subgraphs)
        .build()
        .start()
        .await;

    let response = router
        .send_graphql_request(
            "{ users { id } }",
            None,
            some_header_map!("x-forwarded-for" => "198.51.100.7, 10.0.0.2".to_string()),
        )
        .await;
    assert!(response.status().is_success());

    let http_server_span = otlp_collector
        .wait_for_span_by_hive_kind_one("http.server")
        .await;

    assert_eq!(
        http_server_span
            .attributes
            .get("client.address")
            .map(String::as_str),
        Some("198.51.100.7")
    );
    assert!(http_server_span.attributes.get("client.port").is_none());
}

#[ntex::test]
async fn test_client_ip_trusted_proxies_uses_first_non_trusted_from_right() {
    let otlp_collector = OtlpCollector::start()
        .await
        .expect("Failed to start OTLP collector");
    let otlp_endpoint = otlp_collector.http_traces_endpoint();

    let subgraphs = TestSubgraphs::builder().build().start().await;
    let router = TestRouter::builder()
        .inline_config(format!(
            r#"
          supergraph:
            source: file
            path: supergraph.graphql

          telemetry:
            client_identification:
              ip_header:
                name: x-forwarded-for
                trusted_proxies:
                  - 127.0.0.0/8 # localhost
                  - ::1/128     # localhost
                  - 10.0.0.0/8  # trusted proxy IP range
            tracing:
              exporters:
                - kind: otlp
                  endpoint: {otlp_endpoint}
                  protocol: http
                  batch_processor:
                    scheduled_delay: 50ms
                    max_export_timeout: 2s
      "#,
        ))
        .with_subgraphs(&subgraphs)
        .build()
        .start()
        .await;

    let response = router
        .send_graphql_request(
            "{ users { id } }",
            None,
            // Second IP = trusted proxy IP
            some_header_map!("x-forwarded-for" => "198.51.100.7, 10.0.0.2".to_string()),
        )
        .await;
    assert!(response.status().is_success());

    let http_server_span = otlp_collector
        .wait_for_span_by_hive_kind_one("http.server")
        .await;

    // The client IP should be the first untrusted IP from right to left
    assert_eq!(
        http_server_span
            .attributes
            .get("client.address")
            .map(String::as_str),
        Some("198.51.100.7")
    );
    assert!(http_server_span.attributes.get("client.port").is_none());
}

#[ntex::test]
async fn test_client_ip_trusted_proxies_all_trusted_falls_back_to_left_most() {
    let otlp_collector = OtlpCollector::start()
        .await
        .expect("Failed to start OTLP collector");
    let otlp_endpoint = otlp_collector.http_traces_endpoint();

    let subgraphs = TestSubgraphs::builder().build().start().await;
    let router = TestRouter::builder()
        .inline_config(format!(
            r#"
          supergraph:
            source: file
            path: supergraph.graphql

          telemetry:
            client_identification:
              ip_header:
                name: x-forwarded-for
                trusted_proxies:
                  - 127.0.0.0/8 # localhost
                  - ::1/128     # localhost
                  - 10.0.0.0/8  # trusted proxy IP range
            tracing:
              exporters:
                - kind: otlp
                  endpoint: {otlp_endpoint}
                  protocol: http
                  batch_processor:
                    scheduled_delay: 50ms
                    max_export_timeout: 2s
      "#,
        ))
        .with_subgraphs(&subgraphs)
        .build()
        .start()
        .await;

    let response = router
        .send_graphql_request(
            "{ users { id } }",
            None,
            // all IPs are trusted
            some_header_map!("x-forwarded-for" => "10.1.1.1, 10.2.2.2".to_string()),
        )
        .await;
    assert!(response.status().is_success());

    let http_server_span = otlp_collector
        .wait_for_span_by_hive_kind_one("http.server")
        .await;

    // Since all are trusted then we pick the left-most IP
    assert_eq!(
        http_server_span
            .attributes
            .get("client.address")
            .map(String::as_str),
        Some("10.1.1.1")
    );
    assert!(http_server_span.attributes.get("client.port").is_none());
}
