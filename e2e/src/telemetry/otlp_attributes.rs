use crate::testkit::{
    otel::OtlpCollector, some_header_map, TestRouterBuilder, TestSubgraphsBuilder,
};

/// Verify only deprecated attributes are emitted for deprecated mode
#[ntex::test]
async fn test_deprecated_span_attributes() {
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
              instrumentation:
                spans:
                  mode: deprecated
              propagation:
                trace_context: true
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
        http.flavor: 1.1
        http.host: localhost
        http.method: POST
        http.request_content_length: 45
        http.response_content_length: 86
        http.route: /graphql
        http.status_code: 200
        http.target: /graphql
        http.url: /graphql
        server.port: [port]
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
        http.flavor: 1.1
        http.method: POST
        http.request_content_length: 23
        http.response_content_length: 86
        http.status_code: 200
        http.url: http://[address]:[port]/accounts
        net.peer.name: [address]
        net.peer.port: [port]
        target: hive-router
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
        http.flavor: 1.1
        http.method: POST
        http.request_content_length: 23
        http.response_content_length: 86
        http.status_code: 200
        http.url: http://[address]:[port]/accounts
        net.peer.name: [address]
        net.peer.port: [port]
        target: hive-router
        url.path: /accounts
        url.scheme: http
    "
    );
}

/// Verify both spec-compliant and deprecated attributes are emitted for spec_and_deprecated mode
#[ntex::test]
async fn test_spec_and_deprecated_span_attributes() {
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
              instrumentation:
                spans:
                  mode: spec_and_deprecated
              propagation:
                trace_context: true
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
        http.flavor: 1.1
        http.host: localhost
        http.method: POST
        http.request.body.size: 45
        http.request.method: POST
        http.request_content_length: 45
        http.response.body.size: 86
        http.response.status_code: 200
        http.response_content_length: 86
        http.route: /graphql
        http.status_code: 200
        http.target: /graphql
        http.url: /graphql
        network.protocol.version: 1.1
        server.address: localhost
        server.port: [port]
        target: hive-router
        url.full: /graphql
        url.path: /graphql
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
        http.flavor: 1.1
        http.method: POST
        http.request.body.size: 23
        http.request.method: POST
        http.request_content_length: 23
        http.response.body.size: 86
        http.response.status_code: 200
        http.response_content_length: 86
        http.status_code: 200
        http.url: http://[address]:[port]/accounts
        net.peer.name: [address]
        net.peer.port: [port]
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
        http.flavor: 1.1
        http.method: POST
        http.request.body.size: 23
        http.request.method: POST
        http.request_content_length: 23
        http.response.body.size: 86
        http.response.status_code: 200
        http.response_content_length: 86
        http.status_code: 200
        http.url: http://[address]:[port]/accounts
        net.peer.name: [address]
        net.peer.port: [port]
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

/// Verify default client identification
#[ntex::test]
async fn test_default_client_identification() {
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
        .send_graphql_request(
            "{ users { id } }",
            None,
            some_header_map!(
                http::header::HeaderName::from_static("graphql-client-name") => "e2e",
                http::header::HeaderName::from_static("graphql-client-version") => "tests"
            ),
        )
        .await;

    assert!(res.status().is_success());

    // Wait for exports
    let operation_span = otlp_collector
        .wait_for_span_by_hive_kind_one("graphql.operation")
        .await;

    insta::assert_snapshot!(
      operation_span,
      @r"
    Span: graphql.operation
      Kind: Server
      Status: message='' code='0'
      Attributes:
        graphql.document.hash: 1237612228098794304
        graphql.operation.type: query
        hive.client.name: e2e
        hive.client.version: tests
        hive.graphql.operation.hash: e92177e49c0010d4e52929531ebe30c9
        hive.kind: graphql.operation
        target: hive-router
    "
    );
}

/// Verify custom client identification
#[ntex::test]
async fn test_custom_client_identification() {
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
            client_identification:
              name_header: "x-client-name"
              version_header: "x-client-version"
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
        .send_graphql_request(
            "{ users { id } }",
            None,
            some_header_map!(
                http::header::HeaderName::from_static("x-client-name") => "e2e",
                http::header::HeaderName::from_static("x-client-version") => "tests"
            ),
        )
        .await;

    assert!(res.status().is_success());

    // Wait for exports
    let operation_span = otlp_collector
        .wait_for_span_by_hive_kind_one("graphql.operation")
        .await;

    insta::assert_snapshot!(
      operation_span,
      @r"
    Span: graphql.operation
      Kind: Server
      Status: message='' code='0'
      Attributes:
        graphql.document.hash: 1237612228098794304
        graphql.operation.type: query
        hive.client.name: e2e
        hive.client.version: tests
        hive.graphql.operation.hash: e92177e49c0010d4e52929531ebe30c9
        hive.kind: graphql.operation
        target: hive-router
    "
    );
}

/// Verify default resource attributes
#[ntex::test]
async fn test_default_resource_attributes() {
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

    let res = router
        .send_graphql_request("{ users { id } }", None, None)
        .await;

    assert!(res.status().is_success());

    // Wait for exports
    let all_traces = otlp_collector.wait_for_traces_count(1).await;
    let trace = all_traces.first().unwrap();

    let resource_attributes = trace.merged_resource_attributes();

    assert_eq!(
        resource_attributes.get("service.name"),
        Some(&"hive-router".to_string()),
        "Expected 'service.name' resource attribute to be 'hive-router'"
    );

    assert_eq!(
        resource_attributes.get("telemetry.sdk.language"),
        Some(&"rust".to_string()),
        "Expected 'telemetry.sdk.language' resource attribute to be 'rust'"
    );

    assert_eq!(
        resource_attributes.get("telemetry.sdk.name"),
        Some(&"opentelemetry".to_string()),
        "Expected 'telemetry.sdk.name' resource attribute to be 'opentelemetry'"
    );
}

/// Verify custom resource attributes
#[ntex::test]
async fn test_custom_resource_attributes() {
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
            resource:
              attributes:
                custom.foo: bar
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

    // Wait for exports
    let all_traces = otlp_collector.wait_for_traces_count(1).await;
    let trace = all_traces.first().unwrap();

    let resource_attributes = trace.merged_resource_attributes();

    assert_eq!(
        resource_attributes.get("custom.foo"),
        Some(&"bar".to_string()),
        "Expected 'custom.foo' resource attribute to be 'bar'"
    );
}
