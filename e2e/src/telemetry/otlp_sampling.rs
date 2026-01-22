use ntex::web::test;
use std::time::Duration;

use crate::testkit::{
    init_graphql_request, init_router_from_config_inline,
    otel::{OtlpCollector, TraceParent},
    wait_for_readiness, SubgraphsServer,
};

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

    let req = init_graphql_request("{ users { id } }", None)
        .header("traceparent", upstream_traceparent_sampled.to_string());
    test::call_service(&app.app, req.to_request()).await;

    // Wait for exports
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Verify trace was collected
    let all_traces = otlp_collector.traces().await;
    let trace = all_traces
        .first()
        .expect("Failed to find trace with sampled parent");

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

    // Verify no traces were collected when sampling rate is 0.0
    let all_traces = otlp_collector.traces().await;
    assert!(
        all_traces.is_empty(),
        "No spans should be exported when sampling rate is 0.0, even if parent is sampled"
    );

    app.hold_until_shutdown(Box::new(otlp_collector));
}
