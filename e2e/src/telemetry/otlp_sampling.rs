use std::time::Duration;

use crate::testkit_v2::{
    otel::{OtlpCollector, TraceParent},
    some_header_map, TestRouterBuilder, TestSubgraphsBuilder,
};

/// Verifies parent-based sampler respects upstream sampling decision.
/// Spans sampled according to parent's decision.
#[ntex::test]
async fn test_otlp_parent_based_sampler() {
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
              collect:
                parent_based_sampler: true
                sampling: 1.0
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

    // Upstream says NOT sampled.
    // Even though sampling is 1.0, the parent-based sampler should respect the upstream decision.
    let upstream_trace_id = TraceParent::random_trace_id();
    let upstream_span_id = TraceParent::random_span_id();
    let upstream_traceparent_not_sampled = TraceParent {
        trace_id: &upstream_trace_id,
        span_id: &upstream_span_id,
        sampled: false,
    };

    let res = router
        .send_graphql_request(
            "{ users { id } }",
            None,
            some_header_map!("traceparent" => upstream_traceparent_not_sampled.to_string()),
        )
        .await;

    assert!(res.status().is_success());

    // Wait for exports
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Verify no traces were collected when parent is not sampled
    let all_traces = otlp_collector.traces().await;
    assert!(
        all_traces.is_empty(),
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

    let res = router
        .send_graphql_request(
            "{ users { id } }",
            None,
            some_header_map!("traceparent" => upstream_traceparent_sampled.to_string()),
        )
        .await;

    assert!(res.status().is_success());

    // Verify trace was collected
    let all_traces = otlp_collector.wait_for_traces_count(1).await;
    let trace = all_traces.first().unwrap();

    assert_eq!(
        trace.id, upstream_trace_id,
        "Trace should have correct trace_id"
    );

    // Verify we have spans in the trace
    let spans = &trace.spans;
    assert!(
        !spans.is_empty(),
        "Trace should contain spans when parent is sampled"
    );

    // Find http.server span by hive.kind attribute
    let http_server_span = trace.span_by_hive_kind_one("http.server");

    assert_eq!(
        http_server_span.trace_id, upstream_trace_id,
        "http.server span should have correct trace_id"
    );
}

/// Verify 0.0 sample rate
#[ntex::test]
async fn test_otlp_zero_sample_rate() {
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
              collect:
                sampling: 0.0
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
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Verify no traces were collected when sampling rate is 0.0
    let all_traces = otlp_collector.traces().await;
    assert!(
        all_traces.is_empty(),
        "No spans should be exported when sampling rate is 0.0, even if parent is sampled"
    );
}
