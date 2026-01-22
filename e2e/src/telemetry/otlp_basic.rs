use ntex::web::test;
use std::time::Duration;

use crate::testkit::{
    init_graphql_request, init_router_from_config_inline, otel::OtlpCollector, wait_for_readiness,
    SubgraphsServer,
};

/// Verify OTLP exporter works with HTTP protocol
#[ntex::test]
async fn test_otlp_http_export_with_graphql_request() {
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

    // Wait for exports to be sent
    tokio::time::sleep(Duration::from_millis(60)).await;

    let all_traces = otlp_collector.traces().await;
    let trace = all_traces.first().expect("Failed to get first trace");

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
      Status: message='' code='0'
      Attributes:
        hive.kind: http.server
        http.request.body.size: 45
        http.request.method: POST
        http.route: /graphql
        network.protocol.version: 1.1
        target: hive-router
        url.full: /graphql
        url.path: /graphql
    "
    );

    insta::assert_snapshot!(
      operation_span,
      @r"
    Span: GraphQL Operation
      Kind: Server
      Status: message='' code='0'
      Attributes:
        graphql.document.hash: 1237612228098794304
        graphql.document.text: {users{id}}
        graphql.operation.type: query
        hive.graphql.operation.hash: e92177e49c0010d4e52929531ebe30c9
        hive.kind: graphql.operation
        target: hive-router
    "
    );

    insta::assert_snapshot!(
      parse_span,
      @r"
    Span: GraphQL Document Parsing
      Kind: Internal
      Status: message='' code='0'
      Attributes:
        cache.hit: false
        graphql.document.hash: 1237612228098794304
        graphql.operation.type: query
        hive.kind: graphql.parse
        target: hive-router
    "
    );

    insta::assert_snapshot!(
      validate_span,
      @r"
    Span: GraphQL Document Validation
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
    Span: GraphQL Variable Coercion
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
    Span: GraphQL Document Normalization
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
    Span: GraphQL Operation Planning
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
    Span: GraphQL Operation Execution
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
    Span: GraphQL Subgraph Operation
      Kind: Client
      Status: message='' code='0'
      Attributes:
        graphql.document.hash: 7148583861642513753
        graphql.document.text: {users{id}}
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
        hive.inflight.key: 15555024578502296811
        hive.inflight.role: leader
        hive.kind: http.inflight
        http.request.body.size: 23
        http.request.method: POST
        http.response.body.size: 86
        http.response.status_code: 200
        network.protocol.version: 1.1
        target: hive-router
        url.full: http://0.0.0.0:4200/accounts
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

/// Verify OTLP exporter works with gRPC protocol
#[ntex::test]
async fn test_otlp_grpc_export_with_graphql_request() {
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
      Status: message='' code='0'
      Attributes:
        hive.kind: http.server
        http.request.body.size: 45
        http.request.method: POST
        http.route: /graphql
        network.protocol.version: 1.1
        target: hive-router
        url.full: /graphql
        url.path: /graphql
    "
    );

    insta::assert_snapshot!(
      operation_span,
      @r"
    Span: GraphQL Operation
      Kind: Server
      Status: message='' code='0'
      Attributes:
        graphql.document.hash: 1237612228098794304
        graphql.document.text: {users{id}}
        graphql.operation.type: query
        hive.graphql.operation.hash: e92177e49c0010d4e52929531ebe30c9
        hive.kind: graphql.operation
        target: hive-router
    "
    );

    insta::assert_snapshot!(
      parse_span,
      @r"
    Span: GraphQL Document Parsing
      Kind: Internal
      Status: message='' code='0'
      Attributes:
        cache.hit: false
        graphql.document.hash: 1237612228098794304
        graphql.operation.type: query
        hive.kind: graphql.parse
        target: hive-router
    "
    );

    insta::assert_snapshot!(
      validate_span,
      @r"
    Span: GraphQL Document Validation
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
    Span: GraphQL Variable Coercion
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
    Span: GraphQL Document Normalization
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
    Span: GraphQL Operation Planning
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
    Span: GraphQL Operation Execution
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
    Span: GraphQL Subgraph Operation
      Kind: Client
      Status: message='' code='0'
      Attributes:
        graphql.document.hash: 7148583861642513753
        graphql.document.text: {users{id}}
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
        hive.inflight.key: 15555024578502296811
        hive.inflight.role: leader
        hive.kind: http.inflight
        http.request.body.size: 23
        http.request.method: POST
        http.response.body.size: 86
        http.response.status_code: 200
        network.protocol.version: 1.1
        target: hive-router
        url.full: http://0.0.0.0:4200/accounts
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

/// Verify OTLP exporters do not export telemetry when disabled
#[ntex::test]
async fn test_otlp_disabled() {
    let supergraph_path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("supergraph.graphql");

    let otlp_collector = OtlpCollector::start()
        .await
        .expect("Failed to start OTLP collector");
    let otlp_grpc_endpoint = otlp_collector.grpc_endpoint();
    let otlp_http_endpoint = otlp_collector.http_endpoint();

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
                  protocol: grpc
                  enabled: false
                  batch_processor:
                    scheduled_delay: 50ms
                    max_export_timeout: 50ms
                - kind: otlp
                  endpoint: {}
                  enabled: false
                  protocol: http
                  batch_processor:
                    scheduled_delay: 50ms
                    max_export_timeout: 50ms
      "#,
            supergraph_path.to_str().unwrap(),
            otlp_grpc_endpoint,
            otlp_http_endpoint,
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

    assert_eq!(
        otlp_collector.is_empty().await,
        true,
        "Expected no traces to be exported"
    );

    app.hold_until_shutdown(Box::new(otlp_collector));
}

/// Verify custom headers are sent with HTTP OTLP requests
#[ntex::test]
async fn test_otlp_http_headers() {
    let supergraph_path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("supergraph.graphql");

    let otlp_collector = OtlpCollector::start()
        .await
        .expect("Failed to start OTLP collector");
    let otlp_endpoint = otlp_collector.http_endpoint();

    let app = init_router_from_config_inline(
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

    let req = init_graphql_request("{ users { id } }", None);
    test::call_service(&app.app, req.to_request()).await;

    // Wait for exports
    tokio::time::sleep(Duration::from_millis(100)).await;

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

    let otlp_collector = OtlpCollector::start()
        .await
        .expect("Failed to start OTLP collector");
    let otlp_endpoint = otlp_collector.grpc_endpoint();

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
                  protocol: grpc
                  grpc:
                    metadata:
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

    let req = init_graphql_request("{ users { id } }", None);
    test::call_service(&app.app, req.to_request()).await;

    // Wait for exports
    tokio::time::sleep(Duration::from_millis(100)).await;

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

    app.hold_until_shutdown(Box::new(otlp_collector));
}
