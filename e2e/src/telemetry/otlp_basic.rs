use std::time::Duration;

use crate::testkit::{
    otel::{CollectedSpan, OtlpCollector},
    TestRouterBuilder, TestSubgraphsBuilder,
};

/// Verify OTLP exporter works with HTTP protocol
#[ntex::test]
async fn test_otlp_http_export_with_graphql_request() {
    let supergraph_path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("supergraph.graphql");
    let supergraph_path = supergraph_path.to_str().unwrap();

    let otlp_collector = OtlpCollector::start()
        .await
        .expect("Failed to start OTLP collector");
    let _insta_settings_guard = otlp_collector.insta_filter_settings().bind_to_scope();
    let otlp_endpoint = otlp_collector.http_endpoint();

    let subgraphs = TestSubgraphsBuilder::new().build().start().await;

    let router = TestRouterBuilder::new()
        .inline_config(format!(
            r#"
          supergraph:
            source: file
            path: {supergraph_path}

          telemetry:
            tracing:
              exporters:
                - kind: otlp
                  endpoint: {otlp_endpoint}
                  protocol: http
                  batch_processor:
                    scheduled_delay: 50ms
                    max_export_timeout: 50ms
      "#,
        ))
        .with_subgraphs(&subgraphs)
        .build()
        .start()
        .await;

    let res = router
        .send_graphql_request("{ users { id } }", None, None)
        .await;

    assert!(res.status().is_success());

    // Wait for exports to be sent
    let all_traces = otlp_collector.wait_for_traces_count(1).await;
    let trace = all_traces.first().unwrap();

    let http_server_span = trace.span_by_hive_kind_one("http.server");
    let operation_span = trace.span_by_hive_kind_one("graphql.operation");
    let parse_span = trace.span_by_hive_kind_one("graphql.parse");
    let validate_span = trace.span_by_hive_kind_one("graphql.validate");
    let variable_coercion_span = trace.span_by_hive_kind_one("graphql.variable_coercion");
    let normalization_span = trace.span_by_hive_kind_one("graphql.normalize");
    let plan_span = trace.span_by_hive_kind_one("graphql.plan");
    let execution_span = trace.span_by_hive_kind_one("graphql.execute");
    let subgraph_operation_span = trace.span_by_hive_kind_one("graphql.subgraph.operation");
    let http_inflight_span = trace.span_by_hive_kind_one("http.inflight");
    let http_client_span = trace.span_by_hive_kind_one("http.client");

    insta::assert_snapshot!(
      http_server_span,
      @r"
    Span: http.server
      Kind: Server
      Status: message='' code='1'
      Attributes:
        hive.kind: http.server
        http.request.body.size: 45
        http.request.method: POST
        http.response.body.size: 86
        http.response.status_code: 200
        http.route: /graphql
        network.protocol.version: 1.1
        server.address: localhost
        server.port: [port]
        target: hive-router
        url.full: /graphql
        url.path: /graphql
    "
    );

    insta::assert_snapshot!(
      operation_span,
      @r"
    Span: graphql.operation
      Kind: Server
      Status: message='' code='0'
      Attributes:
        graphql.document.hash: 6258881170828510919
        graphql.operation.type: query
        hive.graphql.operation.hash: e92177e49c0010d4e52929531ebe30c9
        hive.kind: graphql.operation
        target: hive-router
    "
    );

    insta::assert_snapshot!(
      parse_span,
      @r"
    Span: graphql.parse
      Kind: Internal
      Status: message='' code='0'
      Attributes:
        cache.hit: false
        graphql.document.hash: 6258881170828510919
        graphql.operation.type: query
        hive.kind: graphql.parse
        target: hive-router
    "
    );

    insta::assert_snapshot!(
      validate_span,
      @r"
    Span: graphql.validate
      Kind: Internal
      Status: message='' code='0'
      Attributes:
        cache.hit: false
        hive.kind: graphql.validate
        target: hive-router
    "
    );

    insta::assert_snapshot!(
      variable_coercion_span,
      @r"
    Span: graphql.variable_coercion
      Kind: Internal
      Status: message='' code='0'
      Attributes:
        hive.kind: graphql.variable_coercion
        target: hive-router
    "
    );

    insta::assert_snapshot!(
      normalization_span,
      @r"
    Span: graphql.normalize
      Kind: Internal
      Status: message='' code='0'
      Attributes:
        cache.hit: false
        hive.kind: graphql.normalize
        target: hive-router
    "
    );

    insta::assert_snapshot!(
      plan_span,
      @r"
    Span: graphql.plan
      Kind: Internal
      Status: message='' code='0'
      Attributes:
        cache.hit: false
        hive.kind: graphql.plan
        target: hive-router
    "
    );

    insta::assert_snapshot!(
      execution_span,
      @r"
    Span: graphql.execute
      Kind: Internal
      Status: message='' code='0'
      Attributes:
        hive.kind: graphql.execute
        target: hive-router
    "
    );

    insta::assert_snapshot!(
      subgraph_operation_span,
      @r"
    Span: graphql.subgraph.operation
      Kind: Client
      Status: message='' code='0'
      Attributes:
        graphql.document.hash: 7148583861642513753
        graphql.operation.type: query
        hive.graphql.subgraph.name: accounts
        hive.kind: graphql.subgraph.operation
        target: hive-router
    "
    );

    insta::assert_snapshot!(
      http_inflight_span,
      @r"
    Span: http.inflight
      Kind: Internal
      Status: message='' code='1'
      Attributes:
        hive.inflight.key: [random]
        hive.inflight.role: leader
        hive.kind: http.inflight
        http.request.body.size: 23
        http.request.method: POST
        http.response.body.size: 86
        http.response.status_code: 200
        network.protocol.version: 1.1
        server.address: [address]
        server.port: [port]
        target: hive-router
        url.full: http://[address]:[port]/accounts
        url.path: /accounts
        url.scheme: http
    "
    );

    insta::assert_snapshot!(
      http_client_span,
      @r"
    Span: http.client
      Kind: Client
      Status: message='' code='1'
      Attributes:
        hive.kind: http.client
        http.request.body.size: 23
        http.request.method: POST
        http.response.body.size: 86
        http.response.status_code: 200
        network.protocol.version: 1.1
        server.address: [address]
        server.port: [port]
        target: hive-router
        url.full: http://[address]:[port]/accounts
        url.path: /accounts
        url.scheme: http
    "
    );
}

/// Verify OTLP exporter works with gRPC protocol
#[ntex::test]
async fn test_otlp_grpc_export_with_graphql_request() {
    let supergraph_path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("supergraph.graphql");
    let supergraph_path = supergraph_path.to_str().unwrap();

    let otlp_collector = OtlpCollector::start()
        .await
        .expect("Failed to start OTLP collector");
    let _insta_settings_guard = otlp_collector.insta_filter_settings().bind_to_scope();
    let otlp_endpoint = otlp_collector.grpc_endpoint();

    let subgraphs = TestSubgraphsBuilder::new().build().start().await;

    let router = TestRouterBuilder::new()
        .inline_config(format!(
            r#"
          supergraph:
            source: file
            path: {supergraph_path}

          telemetry:
            tracing:
              exporters:
                - kind: otlp
                  endpoint: {otlp_endpoint}
                  protocol: grpc
                  batch_processor:
                    scheduled_delay: 50ms
                    max_export_timeout: 50ms
      "#,
        ))
        .with_subgraphs(&subgraphs)
        .build()
        .start()
        .await;

    let res = router
        .send_graphql_request("{ users { id } }", None, None)
        .await;

    assert!(res.status().is_success());

    // Wait for exports to be sent
    let http_server_span = otlp_collector
        .wait_for_span_by_hive_kind_one("http.server")
        .await;
    let operation_span = otlp_collector
        .wait_for_span_by_hive_kind_one("graphql.operation")
        .await;
    let parse_span = otlp_collector
        .wait_for_span_by_hive_kind_one("graphql.parse")
        .await;
    let validate_span = otlp_collector
        .wait_for_span_by_hive_kind_one("graphql.validate")
        .await;
    let variable_coercion_span = otlp_collector
        .wait_for_span_by_hive_kind_one("graphql.variable_coercion")
        .await;
    let normalization_span = otlp_collector
        .wait_for_span_by_hive_kind_one("graphql.normalize")
        .await;
    let plan_span = otlp_collector
        .wait_for_span_by_hive_kind_one("graphql.plan")
        .await;
    let execution_span = otlp_collector
        .wait_for_span_by_hive_kind_one("graphql.execute")
        .await;
    let subgraph_operation_span = otlp_collector
        .wait_for_span_by_hive_kind_one("graphql.subgraph.operation")
        .await;
    let http_inflight_span = otlp_collector
        .wait_for_span_by_hive_kind_one("http.inflight")
        .await;
    let http_client_span = otlp_collector
        .wait_for_span_by_hive_kind_one("http.client")
        .await;

    insta::assert_snapshot!(
      http_server_span,
      @r"
    Span: http.server
      Kind: Server
      Status: message='' code='1'
      Attributes:
        hive.kind: http.server
        http.request.body.size: 45
        http.request.method: POST
        http.response.body.size: 86
        http.response.status_code: 200
        http.route: /graphql
        network.protocol.version: 1.1
        server.address: localhost
        server.port: [port]
        target: hive-router
        url.full: /graphql
        url.path: /graphql
    "
    );

    insta::assert_snapshot!(
      operation_span,
      @r"
    Span: graphql.operation
      Kind: Server
      Status: message='' code='0'
      Attributes:
        graphql.document.hash: 6258881170828510919
        graphql.operation.type: query
        hive.graphql.operation.hash: e92177e49c0010d4e52929531ebe30c9
        hive.kind: graphql.operation
        target: hive-router
    "
    );

    insta::assert_snapshot!(
      parse_span,
      @r"
    Span: graphql.parse
      Kind: Internal
      Status: message='' code='0'
      Attributes:
        cache.hit: false
        graphql.document.hash: 6258881170828510919
        graphql.operation.type: query
        hive.kind: graphql.parse
        target: hive-router
    "
    );

    insta::assert_snapshot!(
      validate_span,
      @r"
    Span: graphql.validate
      Kind: Internal
      Status: message='' code='0'
      Attributes:
        cache.hit: false
        hive.kind: graphql.validate
        target: hive-router
    "
    );

    insta::assert_snapshot!(
      variable_coercion_span,
      @r"
    Span: graphql.variable_coercion
      Kind: Internal
      Status: message='' code='0'
      Attributes:
        hive.kind: graphql.variable_coercion
        target: hive-router
    "
    );

    insta::assert_snapshot!(
      normalization_span,
      @r"
    Span: graphql.normalize
      Kind: Internal
      Status: message='' code='0'
      Attributes:
        cache.hit: false
        hive.kind: graphql.normalize
        target: hive-router
    "
    );

    insta::assert_snapshot!(
      plan_span,
      @r"
    Span: graphql.plan
      Kind: Internal
      Status: message='' code='0'
      Attributes:
        cache.hit: false
        hive.kind: graphql.plan
        target: hive-router
    "
    );

    insta::assert_snapshot!(
      execution_span,
      @r"
    Span: graphql.execute
      Kind: Internal
      Status: message='' code='0'
      Attributes:
        hive.kind: graphql.execute
        target: hive-router
    "
    );

    insta::assert_snapshot!(
      subgraph_operation_span,
      @r"
    Span: graphql.subgraph.operation
      Kind: Client
      Status: message='' code='0'
      Attributes:
        graphql.document.hash: 7148583861642513753
        graphql.operation.type: query
        hive.graphql.subgraph.name: accounts
        hive.kind: graphql.subgraph.operation
        target: hive-router
    "
    );

    insta::assert_snapshot!(
      http_inflight_span,
      @r"
    Span: http.inflight
      Kind: Internal
      Status: message='' code='1'
      Attributes:
        hive.inflight.key: [random]
        hive.inflight.role: leader
        hive.kind: http.inflight
        http.request.body.size: 23
        http.request.method: POST
        http.response.body.size: 86
        http.response.status_code: 200
        network.protocol.version: 1.1
        server.address: [address]
        server.port: [port]
        target: hive-router
        url.full: http://[address]:[port]/accounts
        url.path: /accounts
        url.scheme: http
    "
    );

    insta::assert_snapshot!(
      http_client_span,
      @r"
    Span: http.client
      Kind: Client
      Status: message='' code='1'
      Attributes:
        hive.kind: http.client
        http.request.body.size: 23
        http.request.method: POST
        http.response.body.size: 86
        http.response.status_code: 200
        network.protocol.version: 1.1
        server.address: [address]
        server.port: [port]
        target: hive-router
        url.full: http://[address]:[port]/accounts
        url.path: /accounts
        url.scheme: http
    "
    );
}

/// Verify OTLP exporters do not export telemetry when disabled
#[ntex::test]
async fn test_otlp_disabled() {
    let supergraph_path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("supergraph.graphql");
    let supergraph_path = supergraph_path.to_str().unwrap();

    let otlp_collector = OtlpCollector::start()
        .await
        .expect("Failed to start OTLP collector");
    let otlp_grpc_endpoint = otlp_collector.grpc_endpoint();
    let otlp_http_endpoint = otlp_collector.http_endpoint();

    let subgraphs = TestSubgraphsBuilder::new().build().start().await;

    let router = TestRouterBuilder::new()
        .inline_config(format!(
            r#"
          supergraph:
            source: file
            path: {supergraph_path}

          telemetry:
            tracing:
              exporters:
                - kind: otlp
                  endpoint: {otlp_grpc_endpoint}
                  protocol: grpc
                  enabled: false
                  batch_processor:
                    scheduled_delay: 50ms
                    max_export_timeout: 50ms
                - kind: otlp
                  endpoint: {otlp_http_endpoint}
                  enabled: false
                  protocol: http
                  batch_processor:
                    scheduled_delay: 50ms
                    max_export_timeout: 50ms
      "#,
        ))
        .with_subgraphs(&subgraphs)
        .build()
        .start()
        .await;

    let res = router
        .send_graphql_request("{ users { id } }", None, None)
        .await;

    assert!(res.status().is_success());

    // Wait for exports to be sent
    tokio::time::sleep(Duration::from_millis(100)).await;

    assert_eq!(
        otlp_collector.is_empty().await,
        true,
        "Expected no traces to be exported"
    );
}

/// Verify custom headers are sent with HTTP OTLP requests
#[ntex::test]
async fn test_otlp_http_headers() {
    let supergraph_path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("supergraph.graphql");
    let supergraph_path = supergraph_path.to_str().unwrap();

    let otlp_collector = OtlpCollector::start()
        .await
        .expect("Failed to start OTLP collector");
    let otlp_endpoint = otlp_collector.http_endpoint();

    let subgraphs = TestSubgraphsBuilder::new().build().start().await;

    let router = TestRouterBuilder::new()
        .inline_config(format!(
            r#"
          supergraph:
            source: file
            path: {supergraph_path}

          telemetry:
            tracing:
              exporters:
                - kind: otlp
                  endpoint: {otlp_endpoint}
                  protocol: http
                  http:
                    headers:
                      custom-header: custom-value
                  batch_processor:
                    scheduled_delay: 50ms
                    max_export_timeout: 50ms
      "#,
        ))
        .with_subgraphs(&subgraphs)
        .build()
        .start()
        .await;

    let res = router
        .send_graphql_request("{ users { id } }", None, None)
        .await;

    assert!(res.status().is_success());

    // Wait for exports
    otlp_collector.wait_for_traces_count(1).await;

    let first_request = otlp_collector
        .request_at(0)
        .await
        .expect("Failed to get first request");
    let custom_header = first_request
        .headers
        .iter()
        .find(|(name, _value)| name == "custom-header");
    assert_eq!(
        custom_header,
        Some(&("custom-header".to_string(), "custom-value".to_string())),
        "Custom header not found in request headers"
    );
}

/// Verify custom metadata is sent with gRPC OTLP requests
#[ntex::test]
async fn test_otlp_grpc_metadata() {
    let supergraph_path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("supergraph.graphql");
    let supergraph_path = supergraph_path.to_str().unwrap();

    let otlp_collector = OtlpCollector::start()
        .await
        .expect("Failed to start OTLP collector");
    let otlp_endpoint = otlp_collector.grpc_endpoint();

    let subgraphs = TestSubgraphsBuilder::new().build().start().await;

    let router = TestRouterBuilder::new()
        .inline_config(format!(
            r#"
          supergraph:
            source: file
            path: {supergraph_path}

          telemetry:
            tracing:
              exporters:
                - kind: otlp
                  endpoint: {otlp_endpoint}
                  protocol: grpc
                  grpc:
                    metadata:
                      custom-header: custom-value
                  batch_processor:
                    scheduled_delay: 50ms
                    max_export_timeout: 50ms
      "#,
        ))
        .with_subgraphs(&subgraphs)
        .build()
        .start()
        .await;

    let res = router
        .send_graphql_request("{ users { id } }", None, None)
        .await;

    assert!(res.status().is_success());

    // Wait for exports
    otlp_collector.wait_for_traces_count(1).await;

    let first_request = otlp_collector
        .request_at(0)
        .await
        .expect("Failed to get first request");
    let custom_header = first_request
        .headers
        .iter()
        .find(|(name, _value)| name == "custom-header");
    assert_eq!(
        custom_header,
        Some(&("custom-header".to_string(), "custom-value".to_string())),
        "Custom header not found in request headers"
    );
}

/// Verify cache.hit attributes are reported correctly
#[ntex::test]
async fn test_otlp_cache_hits() {
    let supergraph_path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("supergraph.graphql");
    let supergraph_path = supergraph_path.to_str().unwrap();

    let otlp_collector = OtlpCollector::start()
        .await
        .expect("Failed to start OTLP collector");
    let _insta_settings_guard = otlp_collector.insta_filter_settings().bind_to_scope();
    let otlp_endpoint = otlp_collector.http_endpoint();

    let subgraphs = TestSubgraphsBuilder::new().build().start().await;

    let router = TestRouterBuilder::new()
        .inline_config(format!(
            r#"
          supergraph:
            source: file
            path: {supergraph_path}

          telemetry:
            tracing:
              exporters:
                - kind: otlp
                  endpoint: {otlp_endpoint}
                  protocol: http
                  batch_processor:
                    scheduled_delay: 50ms
                    max_export_timeout: 50ms
      "#,
        ))
        .with_subgraphs(&subgraphs)
        .build()
        .start()
        .await;

    let res = router
        .send_graphql_request("{ users { id } }", None, None)
        .await;
    assert!(res.status().is_success());

    // Wait for exports to be sent
    otlp_collector.wait_for_traces_count(1).await;

    // Should hit the caches
    let res = router
        .send_graphql_request("{ users { id } }", None, None)
        .await;
    assert!(res.status().is_success());

    // Wait for exports to be sent
    let all_traces = otlp_collector.wait_for_traces_count(2).await;
    let first_trace = all_traces.first().unwrap();
    let second_trace = all_traces.get(1).unwrap();

    let first_parse_span = first_trace.span_by_hive_kind_one("graphql.parse");
    let first_validate_span = first_trace.span_by_hive_kind_one("graphql.validate");
    let first_normalization_span = first_trace.span_by_hive_kind_one("graphql.normalize");
    let first_plan_span = first_trace.span_by_hive_kind_one("graphql.plan");

    let second_parse_span = second_trace.span_by_hive_kind_one("graphql.parse");
    let second_validate_span = second_trace.span_by_hive_kind_one("graphql.validate");
    let second_normalization_span = second_trace.span_by_hive_kind_one("graphql.normalize");
    let second_plan_span = second_trace.span_by_hive_kind_one("graphql.plan");

    fn assert_cache_hit(span: &CollectedSpan) {
        assert_eq!(
            span.attributes.get("cache.hit"),
            Some(&"true".to_string()),
            "Expected cache hit for span '{}'",
            span.name
        );
    }

    fn assert_cache_miss(span: &CollectedSpan) {
        assert_eq!(
            span.attributes.get("cache.hit"),
            Some(&"false".to_string()),
            "Expected cache miss for span '{}'",
            span.name
        );
    }

    assert_cache_miss(first_parse_span);
    assert_cache_miss(first_validate_span);
    assert_cache_miss(first_normalization_span);
    assert_cache_miss(first_plan_span);

    assert_cache_hit(second_parse_span);
    assert_cache_hit(second_validate_span);
    assert_cache_hit(second_normalization_span);
    assert_cache_hit(second_plan_span);
}

/// Verify cache.hit attributes are reported correctly
#[ntex::test]
async fn test_otlp_no_trace_id_collision() {
    let supergraph_path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("supergraph.graphql");
    let supergraph_path = supergraph_path.to_str().unwrap();

    let otlp_collector = OtlpCollector::start()
        .await
        .expect("Failed to start OTLP collector");
    let otlp_endpoint = otlp_collector.http_endpoint();

    let subgraphs = TestSubgraphsBuilder::new().build().start().await;

    let router = TestRouterBuilder::new()
        .inline_config(format!(
            r#"
          supergraph:
            source: file
            path: {supergraph_path}

          telemetry:
            tracing:
              exporters:
                - kind: otlp
                  endpoint: {otlp_endpoint}
                  protocol: http
                  batch_processor:
                    scheduled_delay: 50ms
                    max_export_timeout: 50ms
      "#,
        ))
        .with_subgraphs(&subgraphs)
        .build()
        .start()
        .await;

    futures::future::join_all(vec![
        router.send_graphql_request("{ users { id } }", None, None),
        router.send_graphql_request("{ users { id } }", None, None),
        router.send_graphql_request("{ users { id } }", None, None),
    ])
    .await;

    // Wait for exports to be sent
    let all_traces = otlp_collector.wait_for_traces_count(3).await;
    let first_trace = all_traces.first().unwrap();
    let second_trace = all_traces.get(1).unwrap();
    let third_trace = all_traces.get(2).unwrap();

    assert_ne!(first_trace.id, second_trace.id);
    assert_ne!(second_trace.id, third_trace.id);
    assert_ne!(first_trace.id, third_trace.id);
}
