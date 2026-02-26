use ntex::web::test;
use std::time::Duration;

use crate::testkit::{
    init_graphql_request, init_router_from_config_inline, otel::OtlpCollector, wait_for_readiness,
    SubgraphsServer,
};

/// Verify Hive Console exporter works with HTTP protocol
#[ntex::test]
async fn test_hive_http_export() {
    let supergraph_path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("supergraph.graphql");

    let otlp_collector = OtlpCollector::start()
        .await
        .expect("Failed to start OTLP collector");
    let otlp_endpoint = otlp_collector.http_traces_endpoint();

    let _subgraphs = SubgraphsServer::start().await;

    let token = "your_token_here";
    let target = "my-org/my-project/my-target";

    let mut app = init_router_from_config_inline(
        format!(
            r#"
          supergraph:
            source: file
            path: {}

          telemetry:
            hive:
              token: {}
              target: {}
              tracing:
                endpoint: {}
                enabled: true
                batch_processor:
                  scheduled_delay: 50ms
                  max_export_timeout: 50ms
              usage_reporting:
                enabled: false
      "#,
            supergraph_path.to_str().unwrap(),
            token,
            target,
            otlp_endpoint,
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

    let first_request = otlp_collector
        .request_at(0)
        .await
        .expect("Failed to get first request");

    let authorization_header = first_request
        .headers
        .iter()
        .find(|(key, _val)| key == "authorization")
        .map(|(_key, val)| val.to_string());
    let target_ref_header = first_request
        .headers
        .iter()
        .find(|(key, _val)| key == "x-hive-target-ref")
        .map(|(_key, val)| val.as_str());

    assert_eq!(authorization_header, Some(format!("Bearer {}", token)));
    assert_eq!(target_ref_header, Some(target));

    let all_traces = otlp_collector.traces().await;
    let trace = all_traces.first().expect("Failed to get first trace");

    // Hive Console requires to drop the http.server span
    // and make the graphql.operation the root span.
    assert_eq!(
        trace.has_span_by_hive_kind("http.server"),
        false,
        "Unexpected http.server spans"
    );

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
      operation_span,
      @r"
    Span: graphql.operation
      Kind: Server
      Status: message='' code='0'
      Attributes:
        graphql.document: {users{id}}
        graphql.document.hash: 6258881170828510919
        graphql.operation.type: query
        hive.gateway.operation.subgraph.names: accounts
        hive.graphql: true
        hive.graphql.operation.hash: e92177e49c0010d4e52929531ebe30c9
        hive.kind: graphql.operation
        http.method: POST
        http.route: /graphql
        http.status_code: 200
        http.url: /graphql
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
        graphql.document: {users{id}}
        graphql.document.hash: 7148583861642513753
        graphql.operation.type: query
        hive.graphql.subgraph.name: accounts
        hive.kind: graphql.subgraph.operation
        http.host: 0.0.0.0
        http.method: POST
        http.route: /accounts
        http.status_code: 200
        http.url: http://0.0.0.0:4200/accounts
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
        http.request.body.size: 23
        http.response.body.size: 86
        network.protocol.version: 1.1
        server.port: 4200
        target: hive-router
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
