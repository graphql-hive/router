use ntex::web::test;
use std::time::Duration;

use crate::testkit::{
    init_graphql_request, init_router_from_config_inline,
    otel::{CollectedMetrics, OtlpCollector},
    wait_for_readiness, SubgraphsServer,
};
use hive_router_internal::telemetry::metrics::{labels, names};

/// Verify OTLP metrics exporter works with HTTP protocol
#[ntex::test]
async fn test_otlp_http_metrics_export_with_graphql_request() {
    let supergraph_path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("supergraph.graphql");

    let otlp_collector = OtlpCollector::start()
        .await
        .expect("Failed to start OTLP collector");
    let otlp_endpoint = otlp_collector.http_metrics_endpoint();

    let _subgraphs = SubgraphsServer::start().await;

    let mut app = init_router_from_config_inline(
        format!(
            r#"
          supergraph:
            source: file
            path: {}

          telemetry:
            metrics:
              exporters:
                - kind: otlp
                  endpoint: {}
                  protocol: http
                  interval: 50ms
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

    tokio::time::sleep(Duration::from_millis(120)).await;

    let metrics = otlp_collector.metrics_view().await;
    let assert_histogram_count =
        |metrics: &CollectedMetrics, name: &str, attrs: &[(&str, &str)], expected_count: u64| {
            let (count, sum) = metrics.latest_histogram_count_sum(name, attrs);
            assert_eq!(
                count, expected_count,
                "Expected {name} count to be {expected_count}, got {count}"
            );
            assert!(sum > 0.0, "Expected {name} sum to be > 0, got {sum}");
        };

    let attrs_miss = [(labels::RESULT, labels::CacheResult::Miss.as_str())];

    assert_histogram_count(&metrics, names::PARSE_CACHE_DURATION, &attrs_miss, 1);
    assert_histogram_count(&metrics, names::VALIDATE_CACHE_DURATION, &attrs_miss, 1);
    assert_histogram_count(&metrics, names::NORMALIZE_CACHE_DURATION, &attrs_miss, 1);
    assert_histogram_count(&metrics, names::PLAN_CACHE_DURATION, &attrs_miss, 1);

    let req = init_graphql_request("{ users { id } }", None);
    test::call_service(&app.app, req.to_request()).await;

    tokio::time::sleep(Duration::from_millis(120)).await;

    let metrics = otlp_collector.metrics_view().await;
    let attrs_hit = [(labels::RESULT, labels::CacheResult::Hit.as_str())];

    assert_histogram_count(&metrics, names::PARSE_CACHE_DURATION, &attrs_hit, 1);
    assert_histogram_count(&metrics, names::VALIDATE_CACHE_DURATION, &attrs_hit, 1);
    assert_histogram_count(&metrics, names::NORMALIZE_CACHE_DURATION, &attrs_hit, 1);
    assert_histogram_count(&metrics, names::PLAN_CACHE_DURATION, &attrs_hit, 1);

    let no_attrs: [(&str, &str); 0] = [];
    assert_histogram_count(&metrics, names::PARSE_CACHE_DURATION, &no_attrs, 2);
    assert_histogram_count(&metrics, names::VALIDATE_CACHE_DURATION, &no_attrs, 2);
    assert_histogram_count(&metrics, names::NORMALIZE_CACHE_DURATION, &no_attrs, 2);
    assert_histogram_count(&metrics, names::PLAN_CACHE_DURATION, &no_attrs, 2);

    app.hold_until_shutdown(Box::new(otlp_collector));
}

/// Verify cache size metrics are exported as gauges
#[ntex::test]
async fn test_otlp_cache_size_metrics_exported_as_gauges() {
    let supergraph_path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("supergraph.graphql");

    let otlp_collector = OtlpCollector::start()
        .await
        .expect("Failed to start OTLP collector");
    let otlp_endpoint = otlp_collector.http_metrics_endpoint();

    let _subgraphs = SubgraphsServer::start().await;

    let mut app = init_router_from_config_inline(
        format!(
            r#"
          supergraph:
            source: file
            path: {}

          telemetry:
            metrics:
              exporters:
                - kind: otlp
                  endpoint: {}
                  protocol: http
                  interval: 50ms
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

    tokio::time::sleep(Duration::from_millis(120)).await;

    let metrics = otlp_collector.metrics_view().await;
    let no_attrs: [(&str, &str); 0] = [];

    assert!(
        metrics.has_gauge(names::PARSE_CACHE_SIZE, &no_attrs),
        "Expected {} gauge series to be exported",
        names::PARSE_CACHE_SIZE
    );
    assert!(
        metrics.has_gauge(names::VALIDATE_CACHE_SIZE, &no_attrs),
        "Expected {} gauge series to be exported",
        names::VALIDATE_CACHE_SIZE
    );
    assert!(
        metrics.has_gauge(names::NORMALIZE_CACHE_SIZE, &no_attrs),
        "Expected {} gauge series to be exported",
        names::NORMALIZE_CACHE_SIZE
    );
    assert!(
        metrics.has_gauge(names::PLAN_CACHE_SIZE, &no_attrs),
        "Expected {} gauge series to be exported",
        names::PLAN_CACHE_SIZE
    );

    app.hold_until_shutdown(Box::new(otlp_collector));
}

/// Verify HTTP server semconv metrics are exported for GraphQL handler
#[ntex::test]
async fn test_otlp_http_server_semconv_metrics_for_graphql_handler() {
    let supergraph_path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("supergraph.graphql");

    let otlp_collector = OtlpCollector::start()
        .await
        .expect("Failed to start OTLP collector");
    let otlp_endpoint = otlp_collector.http_metrics_endpoint();

    let _subgraphs = SubgraphsServer::start().await;

    let mut app = init_router_from_config_inline(
        format!(
            r#"
          supergraph:
            source: file
            path: {}

          telemetry:
            metrics:
              exporters:
                - kind: otlp
                  endpoint: {}
                  protocol: http
                  interval: 50ms
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

    tokio::time::sleep(Duration::from_millis(120)).await;

    let metrics = otlp_collector.metrics_view().await;
    let attrs = [
        (labels::HTTP_REQUEST_METHOD, "POST"),
        (labels::HTTP_ROUTE, "/graphql"),
        (labels::HTTP_RESPONSE_STATUS_CODE, "200"),
    ];

    let assert_histogram = |name: &str| {
        let (count, sum) = metrics.latest_histogram_count_sum(name, &attrs);
        assert_eq!(count, 1, "Expected {name} count to be 1, got {count}");
        assert!(sum > 0.0, "Expected {name} sum to be > 0, got {sum}");
    };

    assert_histogram(names::HTTP_SERVER_REQUEST_DURATION);
    assert_histogram(names::HTTP_SERVER_REQUEST_BODY_SIZE);
    assert_histogram(names::HTTP_SERVER_RESPONSE_BODY_SIZE);

    let active_requests = metrics.latest_counter(names::HTTP_SERVER_ACTIVE_REQUESTS, &[]);
    assert_eq!(
        active_requests,
        0.0,
        "Expected {} to be 0, got {active_requests}",
        names::HTTP_SERVER_ACTIVE_REQUESTS
    );

    app.hold_until_shutdown(Box::new(otlp_collector));
}

/// Verify HTTP client semconv metrics are exported for subgraph requests
#[ntex::test]
async fn test_otlp_http_client_semconv_metrics_for_subgraph_request() {
    let supergraph_path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("supergraph.graphql");

    let otlp_collector = OtlpCollector::start()
        .await
        .expect("Failed to start OTLP collector");
    let otlp_endpoint = otlp_collector.http_metrics_endpoint();

    let _subgraphs = SubgraphsServer::start().await;

    let mut app = init_router_from_config_inline(
        format!(
            r#"
          supergraph:
            source: file
            path: {}

          telemetry:
            metrics:
              exporters:
                - kind: otlp
                  endpoint: {}
                  protocol: http
                  interval: 50ms
                  max_export_timeout: 50ms
                  temporality: cumulative
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

    tokio::time::sleep(Duration::from_millis(120)).await;

    let metrics = otlp_collector.metrics_view().await;
    let attrs = [(labels::HTTP_REQUEST_METHOD, "POST")];

    let assert_histogram = |name: &str| {
        let (count, sum) = metrics.latest_histogram_count_sum(name, &attrs);
        assert_eq!(count, 1, "Expected {name} count to be 1, got {count}");
        assert!(sum > 0.0, "Expected {name} sum to be > 0, got {sum}");
    };

    assert_histogram(names::HTTP_CLIENT_REQUEST_DURATION);
    assert_histogram(names::HTTP_CLIENT_REQUEST_BODY_SIZE);
    assert_histogram(names::HTTP_CLIENT_RESPONSE_BODY_SIZE);

    let active_requests = metrics.latest_counter(names::HTTP_CLIENT_ACTIVE_REQUESTS, &[]);
    assert_eq!(
        active_requests,
        0.0,
        "Expected {} to be 0, got {active_requests}",
        names::HTTP_CLIENT_ACTIVE_REQUESTS
    );

    app.hold_until_shutdown(Box::new(otlp_collector));
}
