use ntex::web::test;
use std::time::Duration;
use tracing::info;

use crate::testkit::{
    init_graphql_request, init_router_from_config_inline,
    otel::{OtlpCollector, SpanCollector, TraceParent},
    wait_for_readiness, SubgraphsServer,
};

#[ntex::test]
async fn test_otlp_http_export_with_graphql_request() {
    let supergraph_path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("supergraph.graphql");

    // Start custom OTLP collector server
    let otlp_collector = OtlpCollector::start()
        .await
        .expect("Failed to start OTLP collector");

    let otlp_endpoint = otlp_collector.http_endpoint();
    info!("OTLP HTTP collector started at: {}", otlp_endpoint);

    let subgraphs = SubgraphsServer::start().await;

    // Initialize router with custom OTLP endpoint
    let app = init_router_from_config_inline(
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
    };

    // Make a GraphQL request
    let req = init_graphql_request("{ users { id } }", None)
        .header("traceparent", upstream_traceparent.to_string());
    test::call_service(&app.app, req.to_request()).await;

    // Wait for exports to be sent
    tokio::time::sleep(Duration::from_millis(60)).await;

    let first_request_spans: SpanCollector = otlp_collector
        .spans_from_request(0)
        .await
        .expect("Failed to get spans from first request");

    let operation_spans = first_request_spans.by_hive_kind("graphql.operation");
    assert_eq!(
        operation_spans.len(),
        1,
        "Expected exactly one graphql.operation span"
    );

    let operation_span = operation_spans.first().unwrap();
    // insta::assert_snapshot!(
    //   operation_span,
    //   @r"
    // Span: GraphQL Operation
    //   Kind: Server
    //   Status: message='' code='0'
    //   Attributes:
    //     graphql.document.hash: 1237612228098794304
    //     graphql.document.text: {users{id}}
    //     graphql.operation.type: query
    //     hive.graphql.error.codes: SUBGRAPH_REQUEST_FAILURE
    //     hive.graphql.error.count: 1
    //     hive.graphql.operation.hash: e92177e49c0010d4e52929531ebe30c9
    //     hive.kind: graphql.operation
    //     target: hive-router
    //   Events:
    //     - Failed to execute request to subgraph
    //       error.message: Failed to execute request to subgraph
    //       error.type: SUBGRAPH_REQUEST_FAILURE
    //       hive.error.subgraph_name: accounts
    //       hive.kind: graphql.error
    // "
    // );

    let http_server_spans = first_request_spans.by_hive_kind("http.server");
    assert_eq!(
        http_server_spans.len(),
        1,
        "Expected exactly one http.server span"
    );

    let http_server_span = http_server_spans.first().unwrap();
    // insta::assert_snapshot!(
    //   http_server_span,
    //   @r"
    // Span: http.server
    //   Kind: Server
    //   Status: message='' code='0'
    //   Attributes:
    //     hive.kind: http.server
    //     http.request.body.size: 45
    //     http.request.method: POST
    //     http.route: /graphql
    //     network.protocol.version: 1.1
    //     target: hive-router
    //     url.full: /graphql
    //     url.path: /graphql
    // "
    // );

    let http_client_spans = first_request_spans.by_hive_kind("http.client");
    assert_eq!(
        http_server_spans.len(),
        1,
        "Expected exactly one http.server span"
    );
    let http_client_span = http_client_spans.first().unwrap();

    // insta::assert_snapshot!(
    //   http_client_span,
    //   @r"
    // Span: http.client
    //   Kind: Client
    //   Status: message='' code='0'
    //   Attributes:
    //     hive.kind: http.client
    //     http.request.body.size: 23
    //     http.request.method: POST
    //     network.protocol.version: 1.1
    //     server.address: 0.0.0.0
    //     server.port: 4200
    //     target: hive-router
    //     url.full: http://0.0.0.0:4200/accounts
    //     url.path: /accounts
    //     url.scheme: http
    // "
    // );

    let account_requests = subgraphs
        .get_subgraph_requests_log("accounts")
        .await
        .expect("Expected at least one request to account subgraph");

    assert!(
        !account_requests.is_empty(),
        "Subgraph should receive requests"
    );

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
        "Expected span_id to match"
    );
}

#[ntex::test]
async fn test_otlp_grpc_export_with_graphql_request() {
    let supergraph_path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("supergraph.graphql");

    // Start custom OTLP collector server
    let otlp_collector = OtlpCollector::start()
        .await
        .expect("Failed to start OTLP collector");

    let otlp_endpoint = otlp_collector.grpc_endpoint();
    info!("OTLP gRPC collector started at: {}", otlp_endpoint);

    let subgraphs = SubgraphsServer::start().await;

    // Initialize router with custom OTLP endpoint
    let app = init_router_from_config_inline(
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

    let upstream_trace_id = TraceParent::random_trace_id();
    let upstream_span_id = TraceParent::random_span_id();
    let upstream_traceparent = TraceParent {
        trace_id: &upstream_trace_id,
        span_id: &upstream_span_id,
    };

    // Make a GraphQL request
    let req = init_graphql_request("{ users { id } }", None)
        .header("traceparent", upstream_traceparent.to_string());
    test::call_service(&app.app, req.to_request()).await;

    // Wait for exports to be sent
    tokio::time::sleep(Duration::from_millis(60)).await;

    let first_request_spans: SpanCollector = otlp_collector
        .spans_from_request(0)
        .await
        .expect("Failed to get spans from first request");

    let operation_spans = first_request_spans.by_hive_kind("graphql.operation");
    assert_eq!(
        operation_spans.len(),
        1,
        "Expected exactly one graphql.operation span"
    );

    let operation_span = operation_spans.first().unwrap();
    // insta::assert_snapshot!(
    //   operation_span,
    //   @r"
    // Span: GraphQL Operation
    //   Kind: Server
    //   Status: message='' code='0'
    //   Attributes:
    //     graphql.document.hash: 1237612228098794304
    //     graphql.document.text: {users{id}}
    //     graphql.operation.type: query
    //     hive.graphql.error.codes: SUBGRAPH_REQUEST_FAILURE
    //     hive.graphql.error.count: 1
    //     hive.graphql.operation.hash: e92177e49c0010d4e52929531ebe30c9
    //     hive.kind: graphql.operation
    //     target: hive-router
    //   Events:
    //     - Failed to execute request to subgraph
    //       error.message: Failed to execute request to subgraph
    //       error.type: SUBGRAPH_REQUEST_FAILURE
    //       hive.error.subgraph_name: accounts
    //       hive.kind: graphql.error
    // "
    // );

    let http_server_spans = first_request_spans.by_hive_kind("http.server");
    assert_eq!(
        http_server_spans.len(),
        1,
        "Expected exactly one http.server span"
    );

    let http_server_span = http_server_spans.first().unwrap();
    // insta::assert_snapshot!(
    //   http_server_span,
    //   @r"
    // Span: http.server
    //   Kind: Server
    //   Status: message='' code='0'
    //   Attributes:
    //     hive.kind: http.server
    //     http.request.body.size: 45
    //     http.request.method: POST
    //     http.route: /graphql
    //     network.protocol.version: 1.1
    //     target: hive-router
    //     url.full: /graphql
    //     url.path: /graphql
    // "
    // );

    let http_client_spans = first_request_spans.by_hive_kind("http.client");
    assert_eq!(
        http_server_spans.len(),
        1,
        "Expected exactly one http.server span"
    );
    let http_client_span = http_client_spans.first().unwrap();

    // insta::assert_snapshot!(
    //   http_client_span,
    //   @r"
    // Span: http.client
    //   Kind: Client
    //   Status: message='' code='0'
    //   Attributes:
    //     hive.kind: http.client
    //     http.request.body.size: 23
    //     http.request.method: POST
    //     network.protocol.version: 1.1
    //     server.address: 0.0.0.0
    //     server.port: 4200
    //     target: hive-router
    //     url.full: http://0.0.0.0:4200/accounts
    //     url.path: /accounts
    //     url.scheme: http
    // "
    // );

    let account_requests = subgraphs
        .get_subgraph_requests_log("accounts")
        .await
        .expect("Expected at least one request to account subgraph");

    assert!(
        !account_requests.is_empty(),
        "Subgraph should receive requests"
    );

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
        "Expected span_id to match"
    );
}
