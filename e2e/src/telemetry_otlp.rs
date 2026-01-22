use ntex::web::test;
use std::time::Duration;

use crate::testkit::{
    init_graphql_request, init_router_from_config_inline,
    otel::{OtlpCollector, SpanCollector, TraceParent},
    wait_for_readiness, SubgraphsServer,
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
                  enabled: true
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

    let first_request_spans: SpanCollector = otlp_collector
        .spans_from_request(0)
        .await
        .expect("Failed to get spans from first request");

    let http_server_span = first_request_spans.by_hive_kind_one("http.server");
    let operation_span = first_request_spans.by_hive_kind_one("graphql.operation");
    let parse_span = first_request_spans.by_hive_kind_one("graphql.parse");
    let validate_span = first_request_spans.by_hive_kind_one("graphql.validate");
    let variable_coercion_span = first_request_spans.by_hive_kind_one("graphql.variable_coercion");
    let normalization_span = first_request_spans.by_hive_kind_one("graphql.normalize");
    let plan_span = first_request_spans.by_hive_kind_one("graphql.plan");
    let execution_span = first_request_spans.by_hive_kind_one("graphql.execute");
    let subgraph_operation_span =
        first_request_spans.by_hive_kind_one("graphql.subgraph.operation");
    let http_inflight_span = first_request_spans.by_hive_kind_one("http.inflight");
    let http_client_span = first_request_spans.by_hive_kind_one("http.client");

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
                  enabled: true
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

    let first_request_spans: SpanCollector = otlp_collector
        .spans_from_request(0)
        .await
        .expect("Failed to get spans from first request");

    let http_server_span = first_request_spans.by_hive_kind_one("http.server");
    let operation_span = first_request_spans.by_hive_kind_one("graphql.operation");
    let parse_span = first_request_spans.by_hive_kind_one("graphql.parse");
    let validate_span = first_request_spans.by_hive_kind_one("graphql.validate");
    let variable_coercion_span = first_request_spans.by_hive_kind_one("graphql.variable_coercion");
    let normalization_span = first_request_spans.by_hive_kind_one("graphql.normalize");
    let plan_span = first_request_spans.by_hive_kind_one("graphql.plan");
    let execution_span = first_request_spans.by_hive_kind_one("graphql.execute");
    let subgraph_operation_span =
        first_request_spans.by_hive_kind_one("graphql.subgraph.operation");
    let http_inflight_span = first_request_spans.by_hive_kind_one("http.inflight");
    let http_client_span = first_request_spans.by_hive_kind_one("http.client");

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

/// Verify Trace Context Propagation (traceparent)
/// From upstream to router to subgraph
#[ntex::test]
async fn test_otlp_http_trace_context_propagation() {
    let supergraph_path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("supergraph.graphql");

    let otlp_collector = OtlpCollector::start()
        .await
        .expect("Failed to start OTLP collector");
    let otlp_endpoint = otlp_collector.http_endpoint();

    let subgraphs = SubgraphsServer::start().await;

    let mut app = init_router_from_config_inline(
        format!(
            r#"
          supergraph:
            source: file
            path: {}

          telemetry:
            tracing:
              propagation:
                trace_context: true
              exporters:
                - kind: otlp
                  enabled: true
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

    let upstream_trace_id = TraceParent::random_trace_id();
    let upstream_span_id = TraceParent::random_span_id();
    let upstream_traceparent = TraceParent {
        trace_id: &upstream_trace_id,
        span_id: &upstream_span_id,
        sampled: true,
    };

    let req = init_graphql_request("{ users { id } }", None)
        .header("traceparent", upstream_traceparent.to_string());
    test::call_service(&app.app, req.to_request()).await;

    // Wait for exports to be sent
    tokio::time::sleep(Duration::from_millis(60)).await;

    let first_request_spans: SpanCollector = otlp_collector
        .spans_from_request(0)
        .await
        .expect("Failed to get spans from first request");

    let http_server_span = first_request_spans.by_hive_kind_one("http.server");
    let http_client_span = first_request_spans.by_hive_kind_one("http.client");

    // Verify that http.server has corrent parent span,
    // the one from upstream traceparent
    assert_eq!(
        http_server_span.parent_span_id, upstream_span_id,
        "http.server span should have correct parent_span_id"
    );
    // Verify that http.server has corrent trace id,
    // the one from upstream traceparent
    assert_eq!(
        http_server_span.trace_id, upstream_trace_id,
        "http.server span should have correct trace_id"
    );

    let account_requests = subgraphs
        .get_subgraph_requests_log("accounts")
        .await
        .expect("Expected at least one request to account subgraph");

    assert!(
        !account_requests.is_empty(),
        "Subgraph should receive requests"
    );

    // Verify the router -> subgraph propagation
    let first_account_request = &account_requests[0];
    let subgraph_traceparent = first_account_request
        .headers
        .get("traceparent")
        .and_then(|v| v.to_str().ok())
        .expect("Subgraph request should have traceparent header");
    let downstream_traceparent = TraceParent::parse(subgraph_traceparent);
    assert_eq!(
        downstream_traceparent.trace_id, upstream_traceparent.trace_id,
        "Expected trace_id to match"
    );

    // We expect the subgraph to receive the span id of the http.client,
    // which is the actual span that triggers the http request.
    assert_eq!(
        downstream_traceparent.span_id, http_client_span.id,
        "Expect http_client span to be parent of subgraph's upstram request"
    );
    // Verify that correct trace_id was propagated.
    // It should be the same as the one from the upstream traceparent
    assert_eq!(
        downstream_traceparent.trace_id, upstream_traceparent.trace_id,
        "Expected trace_id to match"
    );

    app.hold_until_shutdown(Box::new(otlp_collector));
}

#[ntex::test]
async fn test_otlp_http_baggage_propagation() {
    let supergraph_path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("supergraph.graphql");

    let otlp_collector = OtlpCollector::start()
        .await
        .expect("Failed to start OTLP collector");
    let otlp_endpoint = otlp_collector.http_endpoint();

    let subgraphs = SubgraphsServer::start().await;

    let mut app = init_router_from_config_inline(
        format!(
            r#"
          supergraph:
            source: file
            path: {}

          telemetry:
            tracing:
              propagation:
                trace_context: true
                baggage: true
              exporters:
                - kind: otlp
                  enabled: true
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

    let upstream_trace_id = TraceParent::random_trace_id();
    let upstream_span_id = TraceParent::random_span_id();
    let upstream_traceparent = TraceParent {
        trace_id: &upstream_trace_id,
        span_id: &upstream_span_id,
        sampled: true,
    };
    // Be aware the key-value pairs are separated by commas and sorted A-Z.
    // That's why we wrote them sorted here,
    // to make sure the subgraph receives the baggage with items in the same order.
    let baggage = "debug=true,user_id=123,tenant=acme";

    let req = init_graphql_request("{ users { id } }", None)
        .header("traceparent", upstream_traceparent.to_string())
        .header("baggage", baggage);
    test::call_service(&app.app, req.to_request()).await;

    // Wait for exports to be sent
    tokio::time::sleep(Duration::from_millis(60)).await;

    let account_requests = subgraphs
        .get_subgraph_requests_log("accounts")
        .await
        .expect("Expected at least one request to account subgraph");

    assert!(
        !account_requests.is_empty(),
        "Subgraph should receive requests"
    );

    // Verify the router -> subgraph baggage propagation
    let first_account_request = &account_requests[0];
    let subgraph_baggage = first_account_request
        .headers
        .get("baggage")
        .and_then(|v| v.to_str().ok())
        .expect("Subgraph request should have baggage header");
    assert_eq!(baggage, subgraph_baggage, "Expected baggage to match");

    app.hold_until_shutdown(Box::new(otlp_collector));
}

#[ntex::test]
async fn test_otlp_http_b3_propagation() {
    let supergraph_path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("supergraph.graphql");

    let otlp_collector = OtlpCollector::start()
        .await
        .expect("Failed to start OTLP collector");
    let otlp_endpoint = otlp_collector.http_endpoint();

    let subgraphs = SubgraphsServer::start().await;

    let mut app = init_router_from_config_inline(
        format!(
            r#"
          supergraph:
            source: file
            path: {}

          telemetry:
            tracing:
              propagation:
                b3: true
              exporters:
                - kind: otlp
                  enabled: true
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

    let upstream_trace_id = TraceParent::random_trace_id();
    let upstream_span_id = TraceParent::random_span_id();
    let req = init_graphql_request("{ users { id } }", None)
        .header("X-B3-TraceId", upstream_trace_id.clone())
        .header("X-B3-SpanId", upstream_span_id.clone())
        .header("X-B3-Sampled", "1");
    test::call_service(&app.app, req.to_request()).await;

    // Wait for exports to be sent
    tokio::time::sleep(Duration::from_millis(60)).await;

    let first_request_spans: SpanCollector = otlp_collector
        .spans_from_request(0)
        .await
        .expect("Failed to get spans from first request");

    let http_server_span = first_request_spans.by_hive_kind_one("http.server");
    let http_client_span = first_request_spans.by_hive_kind_one("http.client");

    // Verify that http.server has corrent parent span,
    // the one from upstream traceparent
    assert_eq!(
        http_server_span.parent_span_id, upstream_span_id,
        "http.server span should have correct parent_span_id"
    );
    // Verify that http.server has corrent trace id,
    // the one from upstream traceparent
    assert_eq!(
        http_server_span.trace_id, upstream_trace_id,
        "http.server span should have correct trace_id"
    );

    let account_requests = subgraphs
        .get_subgraph_requests_log("accounts")
        .await
        .expect("Expected at least one request to account subgraph");

    assert!(
        !account_requests.is_empty(),
        "Subgraph should receive requests"
    );

    // Verify the router -> subgraph propagation
    let first_account_request = &account_requests[0];

    let subgraph_b3_trace_id = first_account_request
        .headers
        .get("x-b3-traceid")
        .and_then(|v| v.to_str().ok())
        .expect("Subgraph request should have X-B3-TraceId header");
    let subgraph_b3_span_id = first_account_request
        .headers
        .get("x-b3-spanid")
        .and_then(|v| v.to_str().ok())
        .expect("Subgraph request should have X-B3-SpanId header");
    let subgraph_b3_sampled = first_account_request
        .headers
        .get("x-b3-sampled")
        .and_then(|v| v.to_str().ok())
        .expect("Subgraph request should have X-B3-Sampled header");

    // We expect the subgraph to receive the span id of the http.client,
    // which is the actual span that triggers the http request.
    assert_eq!(
        subgraph_b3_span_id, http_client_span.id,
        "Expect http_client span to be parent of subgraph's upstram request"
    );
    // Verify that correct trace_id was propagated.
    // It should be the same as the one from the upstream traceparent
    assert_eq!(
        subgraph_b3_trace_id, upstream_trace_id,
        "Expected trace_id to match"
    );

    assert_eq!(subgraph_b3_sampled, "1");

    app.hold_until_shutdown(Box::new(otlp_collector));
}

#[ntex::test]
async fn test_otlp_http_jaeger_propagation() {
    let supergraph_path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("supergraph.graphql");

    let otlp_collector = OtlpCollector::start()
        .await
        .expect("Failed to start OTLP collector");
    let otlp_endpoint = otlp_collector.http_endpoint();

    let subgraphs = SubgraphsServer::start().await;

    let mut app = init_router_from_config_inline(
        format!(
            r#"
          supergraph:
            source: file
            path: {}

          telemetry:
            tracing:
              propagation:
                jaeger: true
              exporters:
                - kind: otlp
                  enabled: true
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

    let upstream_trace_id = TraceParent::random_trace_id();
    let upstream_span_id = TraceParent::random_span_id();
    let req = init_graphql_request("{ users { id } }", None).header(
        "uber-trace-id",
        format!("{}:{}:0:1", upstream_trace_id, upstream_span_id,),
    );
    test::call_service(&app.app, req.to_request()).await;

    // Wait for exports to be sent
    tokio::time::sleep(Duration::from_millis(60)).await;

    let first_request_spans: SpanCollector = otlp_collector
        .spans_from_request(0)
        .await
        .expect("Failed to get spans from first request");

    let http_server_span = first_request_spans.by_hive_kind_one("http.server");
    let http_client_span = first_request_spans.by_hive_kind_one("http.client");

    // Verify that http.server has corrent parent span,
    // the one from upstream traceparent
    assert_eq!(
        http_server_span.parent_span_id, upstream_span_id,
        "http.server span should have correct parent_span_id"
    );
    // Verify that http.server has corrent trace id,
    // the one from upstream traceparent
    assert_eq!(
        http_server_span.trace_id, upstream_trace_id,
        "http.server span should have correct trace_id"
    );

    let account_requests = subgraphs
        .get_subgraph_requests_log("accounts")
        .await
        .expect("Expected at least one request to account subgraph");

    assert!(
        !account_requests.is_empty(),
        "Subgraph should receive requests"
    );

    // Verify the router -> subgraph propagation
    let first_account_request = &account_requests[0];
    let subgraph_jaeger = first_account_request
        .headers
        .get("uber-trace-id")
        .and_then(|v| v.to_str().ok())
        .expect("Subgraph request should have uber-trace-id header");

    let jeager_parts: Vec<&str> = subgraph_jaeger.split(':').collect();
    let downstream_trace_id = jeager_parts[0];
    let downstream_span_id = jeager_parts[1];
    let downstream_flags = jeager_parts[3];

    // Verify that correct trace_id was propagated.
    // It should be the same as the one from the upstream traceparent
    assert_eq!(
        downstream_trace_id, upstream_trace_id,
        "Expected trace_id to match"
    );

    // We expect the subgraph to receive the span id of the http.client,
    // which is the actual span that triggers the http request.
    assert_eq!(
        downstream_span_id, http_client_span.id,
        "Expect http_client span to be parent of subgraph's upstram request"
    );
    assert_eq!(downstream_flags, "1");

    app.hold_until_shutdown(Box::new(otlp_collector));
}

/// Verifies parent-based sampler respects upstream sampling decision.
/// Spans sampled according to parent's decision.
#[ntex::test]
async fn test_otlp_parent_based_sampler() {
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
              collect:
                parent_based_sampler: true
                sampling: 1.0
              exporters:
                - kind: otlp
                  enabled: true
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

    // Upstream says NOT sampled.
    // Even though sampling is 1.0, the parent-based sampler should respect the upstream decision.
    let upstream_trace_id = TraceParent::random_trace_id();
    let upstream_span_id = TraceParent::random_span_id();
    let upstream_traceparent_not_sampled = TraceParent {
        trace_id: &upstream_trace_id,
        span_id: &upstream_span_id,
        sampled: false,
    };

    let req = init_graphql_request("{ users { id } }", None)
        .header("traceparent", upstream_traceparent_not_sampled.to_string());
    test::call_service(&app.app, req.to_request()).await;

    // Wait for exports
    tokio::time::sleep(Duration::from_millis(100)).await;

    let spans = otlp_collector.spans_from_request(0).await;
    assert!(
        spans.is_err(),
        "No spans should be exported when parent is not sampled, even if sampling rate is 1.0"
    );

    // Upstream says sampled
    let upstream_trace_id = TraceParent::random_trace_id();
    let upstream_span_id = TraceParent::random_span_id();
    let upstream_traceparent_sampled = TraceParent {
        trace_id: &upstream_trace_id,
        span_id: &upstream_span_id,
        sampled: true,
    };

    let req = init_graphql_request("{ users { id } }", None)
        .header("traceparent", upstream_traceparent_sampled.to_string());
    test::call_service(&app.app, req.to_request()).await;

    // Wait for exports
    tokio::time::sleep(Duration::from_millis(100)).await;

    let first_request_spans: SpanCollector = otlp_collector
        .spans_from_request(0)
        .await
        .expect("Failed to get spans from request with sampled parent");

    let http_server_span = first_request_spans.by_hive_kind_one("http.server");
    assert_eq!(
        http_server_span.trace_id, upstream_trace_id,
        "http.server span should have correct trace_id"
    );

    app.hold_until_shutdown(Box::new(otlp_collector));
}

/// Verify 0.0 sample rate
#[ntex::test]
async fn test_otlp_zero_sample_rate() {
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
              collect:
                sampling: 0.0
              exporters:
                - kind: stdout
                  enabled: true
                - kind: otlp
                  enabled: true
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

    assert_eq!(
        otlp_collector.is_empty().await,
        true,
        "No spans should be exported when when sampling rate is 0.0, even if parent is sampled"
    );

    app.hold_until_shutdown(Box::new(otlp_collector));
}

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
                  enabled: true
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

    let first_request_spans: SpanCollector = otlp_collector
        .spans_from_request(0)
        .await
        .expect("Failed to get spans from first request");

    let http_server_span = first_request_spans.by_hive_kind_one("http.server");
    let http_inflight_span = first_request_spans.by_hive_kind_one("http.inflight");
    let http_client_span = first_request_spans.by_hive_kind_one("http.client");

    insta::assert_snapshot!(
      http_server_span,
      @r"
    Span: http.server
      Kind: Server
      Status: message='' code='0'
      Attributes:
        hive.kind: http.server
        http.flavor: 1.1
        http.method: POST
        http.request_content_length: 45
        http.route: /graphql
        http.target: /graphql
        http.url: /graphql
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
        http.flavor: 1.1
        http.method: POST
        http.request_content_length: 23
        http.response_content_length: 86
        http.status_code: 200
        http.url: http://0.0.0.0:4200/accounts
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
                  enabled: true
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

    let first_request_spans: SpanCollector = otlp_collector
        .spans_from_request(0)
        .await
        .expect("Failed to get spans from first request");

    let http_server_span = first_request_spans.by_hive_kind_one("http.server");
    let http_inflight_span = first_request_spans.by_hive_kind_one("http.inflight");
    let http_client_span = first_request_spans.by_hive_kind_one("http.client");

    insta::assert_snapshot!(
      http_server_span,
      @r"
    Span: http.server
      Kind: Server
      Status: message='' code='0'
      Attributes:
        hive.kind: http.server
        http.flavor: 1.1
        http.method: POST
        http.request.body.size: 45
        http.request.method: POST
        http.request_content_length: 45
        http.route: /graphql
        http.target: /graphql
        http.url: /graphql
        network.protocol.version: 1.1
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
        hive.inflight.key: 15555024578502296811
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

/// Verify introspection queries are NOT instrumented by default
#[ntex::test]
async fn test_otlp_introspection_disabled_by_default() {
    let supergraph_path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("supergraph.graphql");

    let otlp_collector = OtlpCollector::start()
        .await
        .expect("Failed to start OTLP collector");
    let otlp_endpoint = otlp_collector.http_endpoint();

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
                  enabled: true
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

    let req = init_graphql_request("{ __schema { queryType { name } } }", None);
    test::call_service(&app.app, req.to_request()).await;

    // Wait for potential exports
    tokio::time::sleep(Duration::from_millis(100)).await;

    assert_eq!(
        otlp_collector.is_empty().await,
        true,
        "No spans should be exported for introspection queries by default"
    );

    app.hold_until_shutdown(Box::new(otlp_collector));
}

/// Verify introspection queries are instrumented when explicitly enabled
#[ntex::test]
async fn test_otlp_introspection_enabled() {
    let supergraph_path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("supergraph.graphql");

    let otlp_collector = OtlpCollector::start()
        .await
        .expect("Failed to start OTLP collector");
    let otlp_endpoint = otlp_collector.http_endpoint();

    let mut app = init_router_from_config_inline(
        format!(
            r#"
          supergraph:
            source: file
            path: {}

          telemetry:
            tracing:
              instrumentation:
                introspection: true
              exporters:
                - kind: otlp
                  enabled: true
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

    let req = init_graphql_request("{ __schema { queryType { name } } }", None);
    test::call_service(&app.app, req.to_request()).await;

    // Wait for exports
    tokio::time::sleep(Duration::from_millis(100)).await;

    let first_request_spans: SpanCollector = otlp_collector
        .spans_from_request(0)
        .await
        .expect("Failed to get spans from introspection request when enabled");

    let http_server_span = first_request_spans.by_hive_kind_one("http.server");
    assert_eq!(
        http_server_span.name, "http.server",
        "Should have an http.server span for introspection when enabled"
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
                  enabled: true
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
                  enabled: true
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
                  enabled: true
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

    let first_request_spans = otlp_collector
        .spans_from_request(0)
        .await
        .expect("Failed to get first request");

    let operation_span = first_request_spans.by_hive_kind_one("graphql.operation");

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
                  enabled: true
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

    let first_request_spans = otlp_collector
        .spans_from_request(0)
        .await
        .expect("Failed to get first request");

    let operation_span = first_request_spans.by_hive_kind_one("graphql.operation");

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
                  enabled: true
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

    let first_request_spans = otlp_collector
        .spans_from_request(0)
        .await
        .expect("Failed to get first request");

    let resource_attributes = first_request_spans.merged_resource_attributes();

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
                  enabled: true
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

    let first_request_spans = otlp_collector
        .spans_from_request(0)
        .await
        .expect("Failed to get first request");

    assert_eq!(
        first_request_spans
            .merged_resource_attributes()
            .get("custom.foo"),
        Some(&"bar".to_string()),
        "Expected 'custom.foo' resource attribute to be 'bar'"
    );

    app.hold_until_shutdown(Box::new(otlp_collector));
}
