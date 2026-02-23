use crate::testkit_v2::{
    otel::{Baggage, OtlpCollector, TraceParent},
    some_header_map, TestRouterBuilder, TestSubgraphsBuilder,
};

#[ntex::test]
async fn test_otlp_http_trace_context_propagation() {
    let supergraph_path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("supergraph.graphql");
    let supergraph_path = supergraph_path.to_str().unwrap();

    let mut otlp_collector = OtlpCollector::start()
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
              propagation:
                trace_context: true
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

    let upstream_trace_id = TraceParent::random_trace_id();
    let upstream_span_id = TraceParent::random_span_id();
    let upstream_traceparent = TraceParent {
        trace_id: &upstream_trace_id,
        span_id: &upstream_span_id,
        sampled: true,
    };

    let res = router
        .send_graphql_request(
            "{ users { id } }",
            None,
            some_header_map!("traceparent" => upstream_traceparent.to_string()),
        )
        .await;

    assert!(res.status().is_success());

    // Wait for exports to be sent
    let all_traces = otlp_collector.wait_for_traces().await;
    let trace = all_traces.first().expect("Failed to get first trace");

    let http_server_span = trace.span_by_hive_kind_one("http.server");
    let http_client_span = trace.span_by_hive_kind_one("http.client");

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
        .get_requests_log("accounts")
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
}

#[ntex::test]
async fn test_otlp_http_baggage_propagation() {
    let supergraph_path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("supergraph.graphql");
    let supergraph_path = supergraph_path.to_str().unwrap();

    let mut otlp_collector = OtlpCollector::start()
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
              propagation:
                trace_context: true
                baggage: true
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

    let upstream_trace_id = TraceParent::random_trace_id();
    let upstream_span_id = TraceParent::random_span_id();
    let upstream_traceparent = TraceParent {
        trace_id: &upstream_trace_id,
        span_id: &upstream_span_id,
        sampled: true,
    };
    let upstream_baggage = Baggage::from([
        ("debug".into(), "true".into()),
        ("tenant".into(), "acme".into()),
        ("user_id".into(), "123".into()),
    ]);

    let res = router
        .send_graphql_request(
            "{ users { id } }",
            None,
            some_header_map!(
                "traceparent" => upstream_traceparent.to_string(),
                "baggage" => upstream_baggage.to_string(),
            ),
        )
        .await;

    assert!(res.status().is_success());

    // Wait for exports to be sent
    otlp_collector.wait_for_traces().await;

    let account_requests = subgraphs
        .get_requests_log("accounts")
        .expect("Expected at least one request to account subgraph");

    assert!(
        !account_requests.is_empty(),
        "Subgraph should receive requests"
    );

    // Verify the router -> subgraph baggage propagation
    let first_account_request = &account_requests[0];
    let subgraph_baggage_string = first_account_request
        .headers
        .get("baggage")
        .and_then(|v| v.to_str().ok())
        .expect("Subgraph request should have baggage header");

    let subgraph_baggage = Baggage::from(subgraph_baggage_string);

    assert_eq!(
        upstream_baggage, subgraph_baggage,
        "Expected baggage to match"
    );
}

#[ntex::test]
async fn test_otlp_http_b3_propagation() {
    let supergraph_path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("supergraph.graphql");
    let supergraph_path = supergraph_path.to_str().unwrap();

    let mut otlp_collector = OtlpCollector::start()
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
              propagation:
                b3: true
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

    let upstream_trace_id = TraceParent::random_trace_id();
    let upstream_span_id = TraceParent::random_span_id();

    let res = router
        .send_graphql_request(
            "{ users { id } }",
            None,
            some_header_map!(
                "X-B3-TraceId" => upstream_trace_id.clone(),
                "X-B3-SpanId" => upstream_span_id.clone(),
                "X-B3-Sampled" => "1",
            ),
        )
        .await;

    assert!(res.status().is_success());

    // Wait for exports to be sent
    let all_traces = otlp_collector.wait_for_traces().await;
    let trace = all_traces.first().expect("Failed to get first trace");

    let http_server_span = trace.span_by_hive_kind_one("http.server");
    let http_client_span = trace.span_by_hive_kind_one("http.client");

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
        .get_requests_log("accounts")
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
}

#[ntex::test]
async fn test_otlp_http_jaeger_propagation() {
    let supergraph_path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("supergraph.graphql");
    let supergraph_path = supergraph_path.to_str().unwrap();

    let mut otlp_collector = OtlpCollector::start()
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
              propagation:
                jaeger: true
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

    let upstream_trace_id = TraceParent::random_trace_id();
    let upstream_span_id = TraceParent::random_span_id();

    let res = router
        .send_graphql_request(
            "{ users { id } }",
            None,
            some_header_map!("uber-trace-id" => format!("{}:{}:0:1", upstream_trace_id, upstream_span_id)),
        )
        .await;

    assert!(res.status().is_success());

    // Wait for exports to be sent
    let all_traces = otlp_collector.wait_for_traces().await;
    let trace = all_traces.first().expect("Failed to get first trace");

    let http_server_span = trace.span_by_hive_kind_one("http.server");
    let http_client_span = trace.span_by_hive_kind_one("http.client");

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
        .get_requests_log("accounts")
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
}
