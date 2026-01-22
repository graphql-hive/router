use ntex::web::test;
use std::time::Duration;

use crate::testkit::{
    init_graphql_request, init_router_from_config_inline,
    otel::{OtlpCollector, SpanCollector},
    wait_for_readiness,
};

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
