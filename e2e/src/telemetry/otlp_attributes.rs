use ntex::web::test;
use std::time::Duration;

use crate::testkit::{
    init_graphql_request, init_router_from_config_inline, otel::OtlpCollector, wait_for_readiness,
    SubgraphsServer,
};

/// Verify only deprecated attributes are emitted for deprecated mode
#[ntex::test]
async fn test_deprecated_span_attributes() {
    let supergraph_path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("supergraph.graphql");

    let otlp_collector = OtlpCollector::start()
        .await
        .expect("Failed to start OTLP collector");
    let otlp_endpoint = otlp_collector.grpc_endpoint();

    let _subgraphs = SubgraphsServer::start().await;

    let mut app = init_router_from_config_inline(
        format!(
            r#"
          supergraph:
            source: file
            path: {}

          telemetry:
            tracing:
              instrumentation:
                spans:
                  mode: deprecated
              propagation:
                trace_context: true
              exporters:
                - kind: otlp
                  endpoint: {}
                  protocol: grpc
                  batch_processor:
                    scheduled_delay: 50ms
                    max_export_timeout: 50ms
      "#,
            supergraph_path.to_str().unwrap(),
            otlp_endpoint
        )
        .as_str(),
    )
    .await
    .expect("Failed to initialize router from config file");

    wait_for_readiness(&app.app).await;

    let req = init_graphql_request("{ users { id } }", None);
    test::call_service(&app.app, req.to_request()).await;

    // Wait for exports to be sent
    tokio::time::sleep(Duration::from_millis(60)).await;

    let all_traces = otlp_collector.traces().await;
    let trace = all_traces.first().expect("Failed to get first trace");

    let http_server_span = trace.span_by_hive_kind_one("http.server");
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
        http.flavor: 1.1
        http.method: POST
        http.request_content_length: 28
        http.response_content_length: 86
        http.route: /graphql
        http.status_code: 200
        http.target: /graphql
        http.url: /graphql
        target: hive-router
    "
    );

    insta::with_settings!({filters => vec![
      (r"(hive\.inflight\.key:\s+)\d+", "$1[random]"),
    ]}, {
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
        http.url: http://0.0.0.0:4200/accounts
        net.peer.name: 0.0.0.0
        net.peer.port: 4200
        target: hive-router
        url.path: /accounts
        url.scheme: http
    "
    );
    });

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
        http.url: http://0.0.0.0:4200/accounts
        net.peer.name: 0.0.0.0
        net.peer.port: 4200
        target: hive-router
        url.path: /accounts
        url.scheme: http
    "
    );

    app.hold_until_shutdown(Box::new(otlp_collector));
}

/// Verify both spec-compliant and deprecated attributes are emitted for spec_and_deprecated mode
#[ntex::test]
async fn test_spec_and_deprecated_span_attributes() {
    let supergraph_path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("supergraph.graphql");

    let otlp_collector = OtlpCollector::start()
        .await
        .expect("Failed to start OTLP collector");
    let otlp_endpoint = otlp_collector.grpc_endpoint();

    let _subgraphs = SubgraphsServer::start().await;

    let mut app = init_router_from_config_inline(
        format!(
            r#"
          supergraph:
            source: file
            path: {}

          telemetry:
            tracing:
              instrumentation:
                spans:
                  mode: spec_and_deprecated
              propagation:
                trace_context: true
              exporters:
                - kind: otlp
                  endpoint: {}
                  protocol: grpc
                  batch_processor:
                    scheduled_delay: 50ms
                    max_export_timeout: 50ms
      "#,
            supergraph_path.to_str().unwrap(),
            otlp_endpoint
        )
        .as_str(),
    )
    .await
    .expect("Failed to initialize router from config file");

    wait_for_readiness(&app.app).await;

    let req = init_graphql_request("{ users { id } }", None);
    test::call_service(&app.app, req.to_request()).await;

    // Wait for exports to be sent
    tokio::time::sleep(Duration::from_millis(60)).await;

    let all_traces = otlp_collector.traces().await;
    let trace = all_traces.first().expect("Failed to get first trace");

    let http_server_span = trace.span_by_hive_kind_one("http.server");
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
        http.flavor: 1.1
        http.method: POST
        http.request.body.size: 28
        http.request.method: POST
        http.request_content_length: 28
        http.response.body.size: 86
        http.response.status_code: 200
        http.response_content_length: 86
        http.route: /graphql
        http.status_code: 200
        http.target: /graphql
        http.url: /graphql
        network.protocol.version: 1.1
        target: hive-router
        url.full: /graphql
        url.path: /graphql
    "
    );

    insta::with_settings!({filters => vec![
      (r"(hive\.inflight\.key:\s+)\d+", "$1[random]"),
    ]}, {
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
        http.url: http://0.0.0.0:4200/accounts
        net.peer.name: 0.0.0.0
        net.peer.port: 4200
        network.protocol.version: 1.1
        server.address: 0.0.0.0
        server.port: 4200
        target: hive-router
        url.full: http://0.0.0.0:4200/accounts
        url.path: /accounts
        url.scheme: http
    "
    );
    });

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
        http.url: http://0.0.0.0:4200/accounts
        net.peer.name: 0.0.0.0
        net.peer.port: 4200
        network.protocol.version: 1.1
        server.address: 0.0.0.0
        server.port: 4200
        target: hive-router
        url.full: http://0.0.0.0:4200/accounts
        url.path: /accounts
        url.scheme: http
    "
    );

    app.hold_until_shutdown(Box::new(otlp_collector));
}

/// Verify default client identification
#[ntex::test]
async fn test_default_client_identification() {
    let supergraph_path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("supergraph.graphql");

    let otlp_collector = OtlpCollector::start()
        .await
        .expect("Failed to start OTLP collector");
    let otlp_endpoint = otlp_collector.http_endpoint();

    let _subgraphs = SubgraphsServer::start().await;

    let mut app = init_router_from_config_inline(
        format!(
            r#"
          supergraph:
            source: file
            path: {}

          telemetry:
            tracing:
              exporters:
                - kind: otlp
                  endpoint: {}
                  protocol: http
                  http:
                    headers:
                      custom-header: custom-value
                  batch_processor:
                    scheduled_delay: 50ms
                    max_export_timeout: 50ms
      "#,
            supergraph_path.to_str().unwrap(),
            otlp_endpoint
        )
        .as_str(),
    )
    .await
    .expect("Failed to initialize router from config file");

    wait_for_readiness(&app.app).await;

    let req = init_graphql_request("{ users { id } }", None)
        .header("graphql-client-name", "e2e")
        .header("graphql-client-version", "tests");
    test::call_service(&app.app, req.to_request()).await;

    // Wait for exports
    tokio::time::sleep(Duration::from_millis(100)).await;

    let all_traces = otlp_collector.traces().await;
    let trace = all_traces.first().expect("Failed to get first trace");

    let operation_span = trace.span_by_hive_kind_one("graphql.operation");

    insta::assert_snapshot!(
      operation_span,
      @r"
    Span: graphql.operation
      Kind: Server
      Status: message='' code='0'
      Attributes:
        graphql.document.hash: 6258881170828510919
        graphql.operation.type: query
        hive.client.name: e2e
        hive.client.version: tests
        hive.graphql.operation.hash: e92177e49c0010d4e52929531ebe30c9
        hive.kind: graphql.operation
        target: hive-router
    "
    );

    app.hold_until_shutdown(Box::new(otlp_collector));
}

/// Verify custom client identification
#[ntex::test]
async fn test_custom_client_identification() {
    let supergraph_path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("supergraph.graphql");

    let otlp_collector = OtlpCollector::start()
        .await
        .expect("Failed to start OTLP collector");
    let otlp_endpoint = otlp_collector.http_endpoint();

    let _subgraphs = SubgraphsServer::start().await;

    let mut app = init_router_from_config_inline(
        format!(
            r#"
          supergraph:
            source: file
            path: {}

          telemetry:
            client_identification:
              name_header: "x-client-name"
              version_header: "x-client-version"
            tracing:
              exporters:
                - kind: otlp
                  endpoint: {}
                  protocol: http
                  http:
                    headers:
                      custom-header: custom-value
                  batch_processor:
                    scheduled_delay: 50ms
                    max_export_timeout: 50ms
      "#,
            supergraph_path.to_str().unwrap(),
            otlp_endpoint
        )
        .as_str(),
    )
    .await
    .expect("Failed to initialize router from config file");

    wait_for_readiness(&app.app).await;

    let req = init_graphql_request("{ users { id } }", None)
        .header("x-client-name", "e2e")
        .header("x-client-version", "tests");
    test::call_service(&app.app, req.to_request()).await;

    // Wait for exports
    tokio::time::sleep(Duration::from_millis(100)).await;

    let all_traces = otlp_collector.traces().await;
    let trace = all_traces.first().expect("Failed to get first trace");

    let operation_span = trace.span_by_hive_kind_one("graphql.operation");

    insta::assert_snapshot!(
      operation_span,
      @r"
    Span: graphql.operation
      Kind: Server
      Status: message='' code='0'
      Attributes:
        graphql.document.hash: 6258881170828510919
        graphql.operation.type: query
        hive.client.name: e2e
        hive.client.version: tests
        hive.graphql.operation.hash: e92177e49c0010d4e52929531ebe30c9
        hive.kind: graphql.operation
        target: hive-router
    "
    );

    app.hold_until_shutdown(Box::new(otlp_collector));
}

/// Verify default resource attributes
#[ntex::test]
async fn test_default_resource_attributes() {
    let supergraph_path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("supergraph.graphql");

    let otlp_collector = OtlpCollector::start()
        .await
        .expect("Failed to start OTLP collector");
    let otlp_endpoint = otlp_collector.http_endpoint();

    let _subgraphs = SubgraphsServer::start().await;

    let mut app = init_router_from_config_inline(
        format!(
            r#"
          supergraph:
            source: file
            path: {}

          telemetry:
            tracing:
              exporters:
                - kind: otlp
                  endpoint: {}
                  protocol: http
                  batch_processor:
                    scheduled_delay: 50ms
                    max_export_timeout: 50ms
      "#,
            supergraph_path.to_str().unwrap(),
            otlp_endpoint
        )
        .as_str(),
    )
    .await
    .expect("Failed to initialize router from config file");

    wait_for_readiness(&app.app).await;

    let req = init_graphql_request("{ users { id } }", None);
    test::call_service(&app.app, req.to_request()).await;

    // Wait for exports
    tokio::time::sleep(Duration::from_millis(100)).await;

    let all_traces = otlp_collector.traces().await;
    let trace = all_traces.first().expect("Failed to get first trace");

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

    app.hold_until_shutdown(Box::new(otlp_collector));
}

/// Verify custom resource attributes
#[ntex::test]
async fn test_custom_resource_attributes() {
    let supergraph_path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("supergraph.graphql");

    let otlp_collector = OtlpCollector::start()
        .await
        .expect("Failed to start OTLP collector");
    let otlp_endpoint = otlp_collector.http_endpoint();

    let _subgraphs = SubgraphsServer::start().await;

    let mut app = init_router_from_config_inline(
        format!(
            r#"
          supergraph:
            source: file
            path: {}

          telemetry:
            resource:
              attributes:
                custom.foo: bar
            tracing:
              exporters:
                - kind: otlp
                  endpoint: {}
                  protocol: http
                  batch_processor:
                    scheduled_delay: 50ms
                    max_export_timeout: 50ms
      "#,
            supergraph_path.to_str().unwrap(),
            otlp_endpoint
        )
        .as_str(),
    )
    .await
    .expect("Failed to initialize router from config file");

    wait_for_readiness(&app.app).await;

    let req = init_graphql_request("{ users { id } }", None);
    test::call_service(&app.app, req.to_request()).await;

    // Wait for exports
    tokio::time::sleep(Duration::from_millis(100)).await;

    let all_traces = otlp_collector.traces().await;
    let trace = all_traces.first().expect("Failed to get first trace");

    let resource_attributes = trace.merged_resource_attributes();

    assert_eq!(
        resource_attributes.get("custom.foo"),
        Some(&"bar".to_string()),
        "Expected 'custom.foo' resource attribute to be 'bar'"
    );

    app.hold_until_shutdown(Box::new(otlp_collector));
}
