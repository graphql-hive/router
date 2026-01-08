use ntex::web::test;
use std::collections::BTreeSet;
use std::time::Duration;

use crate::testkit::{
    init_graphql_request, init_router_from_config_inline,
    otel::{CollectedMetrics, OtlpCollector},
    wait_for_readiness, SubgraphsServer,
};
use hive_router_internal::telemetry::metrics::catalog::{labels, labels_for, names, values};

async fn wait_for_metrics_export() {
    tokio::time::sleep(Duration::from_millis(100)).await;
}

fn assert_counter_eq(
    metrics: &CollectedMetrics,
    name: &str,
    attrs: &[(&str, &str)],
    expected: f64,
) {
    let count = metrics.latest_counter(name, attrs);
    assert_eq!(
        count, expected,
        "Expected {name} counter to be {expected}, got {count}"
    );
}

fn assert_histogram_count(
    metrics: &CollectedMetrics,
    name: &str,
    attrs: &[(&str, &str)],
    expected_count: u64,
) {
    let (count, sum) = metrics.latest_histogram_count_sum(name, attrs);
    assert_eq!(
        count, expected_count,
        "Expected {name} count to be {expected_count}, got {count}"
    );
    assert!(sum > 0.0, "Expected {name} sum to be > 0, got {sum}");
}

fn assert_metric_has_attrs(metrics: &CollectedMetrics, name: &str, ignore: &[&str]) {
    let actual = metrics.latest_attribute_names(name);
    let expected: BTreeSet<String> = labels_for(name)
        .unwrap_or_else(|| panic!("No labels defined for metric {name}"))
        .iter()
        .filter(|label| !ignore.contains(label))
        .map(|label| label.to_string())
        .collect();
    for attr in expected {
        assert!(
            actual.contains(&attr),
            "Missing attribute {attr} for metric {name}"
        );
    }
}

/// Ensures OTLP metric export works end-to-end for GraphQL traffic.
///
/// This also validates cache duration behavior by asserting a miss on the first request and
/// a hit on the second request for all cache layers.
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
                  interval: 30ms
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

    wait_for_metrics_export().await;

    let metrics = otlp_collector.metrics_view().await;

    let attrs_miss = [(labels::RESULT, values::CacheResult::Miss.as_str())];

    assert_histogram_count(&metrics, names::PARSE_CACHE_DURATION, &attrs_miss, 1);
    assert_histogram_count(&metrics, names::VALIDATE_CACHE_DURATION, &attrs_miss, 1);
    assert_histogram_count(&metrics, names::NORMALIZE_CACHE_DURATION, &attrs_miss, 1);
    assert_histogram_count(&metrics, names::PLAN_CACHE_DURATION, &attrs_miss, 1);

    let req = init_graphql_request("{ users { id } }", None);
    test::call_service(&app.app, req.to_request()).await;

    wait_for_metrics_export().await;

    let metrics = otlp_collector.metrics_view().await;
    let attrs_hit = [(labels::RESULT, values::CacheResult::Hit.as_str())];

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
                  interval: 30ms
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

    wait_for_metrics_export().await;

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

/// Ensures HTTP server semconv metrics are emitted for GraphQL requests.
///
/// Happy-path assertions verify GraphQL labels are present and `error.type` is omitted.
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
                  interval: 30ms
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

    let req = init_graphql_request("query UsersQuery { users { id } }", None);
    test::call_service(&app.app, req.to_request()).await;

    wait_for_metrics_export().await;

    let metrics = otlp_collector.metrics_view().await;
    let attrs = [
        (labels::HTTP_REQUEST_METHOD, "POST"),
        (labels::HTTP_ROUTE, "/graphql"),
        (labels::HTTP_RESPONSE_STATUS_CODE, "200"),
        (labels::GRAPHQL_OPERATION_NAME, "UsersQuery"),
        (labels::GRAPHQL_OPERATION_TYPE, "query"),
        (
            labels::GRAPHQL_RESPONSE_STATUS,
            values::GraphQLResponseStatus::Ok.as_str(),
        ),
    ];

    assert_histogram_count(&metrics, names::HTTP_SERVER_REQUEST_DURATION, &attrs, 1);
    assert_histogram_count(&metrics, names::HTTP_SERVER_REQUEST_BODY_SIZE, &attrs, 1);
    assert_histogram_count(&metrics, names::HTTP_SERVER_RESPONSE_BODY_SIZE, &attrs, 1);
    assert_counter_eq(&metrics, names::HTTP_SERVER_ACTIVE_REQUESTS, &[], 0.0);

    let attrs = metrics.latest_attribute_names(names::HTTP_SERVER_REQUEST_DURATION);
    assert!(
        !attrs.contains(labels::ERROR_TYPE),
        "Expected {} to be absent on happy path",
        labels::ERROR_TYPE
    );

    app.hold_until_shutdown(Box::new(otlp_collector));
}

/// Ensures HTTP client semconv metrics are emitted for outbound subgraph requests.
///
/// Happy-path assertions verify `graphql.response.status=ok` and absence of `error.type`.
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
                  interval: 30ms
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

    wait_for_metrics_export().await;

    let metrics = otlp_collector.metrics_view().await;
    let attrs = [
        (labels::HTTP_REQUEST_METHOD, "POST"),
        (
            labels::GRAPHQL_RESPONSE_STATUS,
            values::GraphQLResponseStatus::Ok.as_str(),
        ),
    ];

    assert_histogram_count(&metrics, names::HTTP_CLIENT_REQUEST_DURATION, &attrs, 1);
    assert_histogram_count(&metrics, names::HTTP_CLIENT_REQUEST_BODY_SIZE, &attrs, 1);
    assert_histogram_count(&metrics, names::HTTP_CLIENT_RESPONSE_BODY_SIZE, &attrs, 1);
    assert_counter_eq(&metrics, names::HTTP_CLIENT_ACTIVE_REQUESTS, &[], 0.0);

    let attrs = metrics.latest_attribute_names(names::HTTP_CLIENT_REQUEST_DURATION);
    assert!(
        !attrs.contains(labels::ERROR_TYPE),
        "Expected {} to be absent on happy path",
        labels::ERROR_TYPE
    );

    app.hold_until_shutdown(Box::new(otlp_collector));
}

/// Ensures every declared metric exposes its expected attribute keys on a non-happy path.
///
/// We intentionally use a path where error labels may appear to validate the full catalog shape.
#[ntex::test]
async fn test_otlp_all_metrics_path_attribute_names() {
    let supergraph_path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("supergraph.graphql");

    let otlp_collector = OtlpCollector::start()
        .await
        .expect("Failed to start OTLP collector");
    let otlp_endpoint = otlp_collector.http_metrics_endpoint();

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
                  interval: 30ms
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

    wait_for_metrics_export().await;

    let metrics = otlp_collector.metrics_view().await;

    for (name, ignore) in [
        (names::SUPERGRAPH_POLL_DURATION, &[][..]),
        (names::SUPERGRAPH_PROCESS_DURATION, &[][..]),
        (names::SUPERGRAPH_POLL_TOTAL, &[][..]),
        (
            names::HTTP_SERVER_REQUEST_DURATION,
            &[labels::ERROR_TYPE][..],
        ),
        (
            names::HTTP_SERVER_REQUEST_BODY_SIZE,
            &[labels::ERROR_TYPE][..],
        ),
        (
            names::HTTP_SERVER_RESPONSE_BODY_SIZE,
            &[labels::ERROR_TYPE][..],
        ),
        (
            names::HTTP_SERVER_ACTIVE_REQUESTS,
            &[labels::ERROR_TYPE][..],
        ),
        (
            names::HTTP_CLIENT_REQUEST_DURATION,
            &[labels::HTTP_RESPONSE_STATUS_CODE][..],
        ),
        (
            names::HTTP_CLIENT_REQUEST_BODY_SIZE,
            &[labels::HTTP_RESPONSE_STATUS_CODE][..],
        ),
        (
            names::HTTP_CLIENT_RESPONSE_BODY_SIZE,
            &[labels::HTTP_RESPONSE_STATUS_CODE][..],
        ),
        (
            names::HTTP_CLIENT_ACTIVE_REQUESTS,
            &[labels::HTTP_RESPONSE_STATUS_CODE][..],
        ),
        (names::PARSE_CACHE_REQUESTS_TOTAL, &[][..]),
        (names::PARSE_CACHE_DURATION, &[][..]),
        (names::PARSE_CACHE_SIZE, &[][..]),
        (names::VALIDATE_CACHE_REQUESTS_TOTAL, &[][..]),
        (names::VALIDATE_CACHE_DURATION, &[][..]),
        (names::VALIDATE_CACHE_SIZE, &[][..]),
        (names::NORMALIZE_CACHE_REQUESTS_TOTAL, &[][..]),
        (names::NORMALIZE_CACHE_DURATION, &[][..]),
        (names::NORMALIZE_CACHE_SIZE, &[][..]),
        (names::PLAN_CACHE_REQUESTS_TOTAL, &[][..]),
        (names::PLAN_CACHE_DURATION, &[][..]),
        (names::PLAN_CACHE_SIZE, &[][..]),
    ] {
        assert_metric_has_attrs(&metrics, name, ignore);
    }

    app.hold_until_shutdown(Box::new(otlp_collector));
}

/// Ensures metric attribute keys remain correct on happy path and error-only signals stay absent.
#[ntex::test]
async fn test_otlp_all_metrics_happy_path_attribute_names() {
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
                  interval: 30ms
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

    wait_for_metrics_export().await;

    let metrics = otlp_collector.metrics_view().await;

    assert!(
        !metrics.has_counter(names::GRAPHQL_ERRORS_TOTAL, &[]),
        "Expected {} to be absent on happy path",
        names::GRAPHQL_ERRORS_TOTAL
    );

    for (name, ignore) in [
        (names::SUPERGRAPH_POLL_DURATION, &[][..]),
        (names::SUPERGRAPH_PROCESS_DURATION, &[][..]),
        (names::SUPERGRAPH_POLL_TOTAL, &[][..]),
        (
            names::HTTP_SERVER_REQUEST_DURATION,
            &[labels::ERROR_TYPE][..],
        ),
        (
            names::HTTP_SERVER_REQUEST_BODY_SIZE,
            &[labels::ERROR_TYPE][..],
        ),
        (
            names::HTTP_SERVER_RESPONSE_BODY_SIZE,
            &[labels::ERROR_TYPE][..],
        ),
        (names::HTTP_SERVER_ACTIVE_REQUESTS, &[][..]),
        (
            names::HTTP_CLIENT_REQUEST_DURATION,
            &[labels::ERROR_TYPE][..],
        ),
        (
            names::HTTP_CLIENT_REQUEST_BODY_SIZE,
            &[labels::ERROR_TYPE][..],
        ),
        (
            names::HTTP_CLIENT_RESPONSE_BODY_SIZE,
            &[labels::ERROR_TYPE][..],
        ),
        (names::HTTP_CLIENT_ACTIVE_REQUESTS, &[][..]),
        (names::PARSE_CACHE_REQUESTS_TOTAL, &[][..]),
        (names::PARSE_CACHE_DURATION, &[][..]),
        (names::PARSE_CACHE_SIZE, &[][..]),
        (names::VALIDATE_CACHE_REQUESTS_TOTAL, &[][..]),
        (names::VALIDATE_CACHE_DURATION, &[][..]),
        (names::VALIDATE_CACHE_SIZE, &[][..]),
        (names::NORMALIZE_CACHE_REQUESTS_TOTAL, &[][..]),
        (names::NORMALIZE_CACHE_DURATION, &[][..]),
        (names::NORMALIZE_CACHE_SIZE, &[][..]),
        (names::PLAN_CACHE_REQUESTS_TOTAL, &[][..]),
        (names::PLAN_CACHE_DURATION, &[][..]),
        (names::PLAN_CACHE_SIZE, &[][..]),
    ] {
        assert_metric_has_attrs(&metrics, name, ignore);
    }

    app.hold_until_shutdown(Box::new(otlp_collector));
}

/// Ensures an instrument can be fully disabled via telemetry instrumentation config.
///
/// The test verifies only the targeted metric is disabled, while related metrics remain enabled.
#[ntex::test]
async fn test_otlp_metric_can_be_disabled_via_instrumentation_config() {
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
                  interval: 30ms
                  max_export_timeout: 50ms
              instrumentation:
                instruments:
                  http.server.request.duration: false
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

    wait_for_metrics_export().await;

    let metrics = otlp_collector.metrics_view().await;
    assert!(
        !metrics.has_histogram(names::HTTP_SERVER_REQUEST_DURATION, &[]),
        "Expected {} histogram to be disabled",
        names::HTTP_SERVER_REQUEST_DURATION
    );
    assert!(
        metrics.has_histogram(names::HTTP_SERVER_REQUEST_BODY_SIZE, &[]),
        "Expected {} histogram to stay enabled",
        names::HTTP_SERVER_REQUEST_BODY_SIZE
    );

    app.hold_until_shutdown(Box::new(otlp_collector));
}

/// Ensures per-metric attribute opt-out drops only the requested label.
///
/// Metric emission must stay enabled and other labels must still be present.
#[ntex::test]
async fn test_otlp_metric_attribute_can_be_opted_out() {
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
                  interval: 30ms
                  max_export_timeout: 50ms
              instrumentation:
                instruments:
                  http.server.request.duration:
                    attributes:
                      http.route: false
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

    wait_for_metrics_export().await;

    let metrics = otlp_collector.metrics_view().await;

    assert!(
        metrics.has_histogram(names::HTTP_SERVER_REQUEST_DURATION, &[]),
        "Expected {} histogram to stay enabled",
        names::HTTP_SERVER_REQUEST_DURATION
    );

    let attrs = metrics.latest_attribute_names(names::HTTP_SERVER_REQUEST_DURATION);
    assert!(
        !attrs.contains(labels::HTTP_ROUTE),
        "Expected {} attribute to be dropped",
        labels::HTTP_ROUTE
    );

    assert_metric_has_attrs(
        &metrics,
        names::HTTP_SERVER_REQUEST_DURATION,
        &[labels::HTTP_ROUTE, labels::ERROR_TYPE],
    );

    app.hold_until_shutdown(Box::new(otlp_collector));
}

/// Ensures setting an attribute override to `true` behaves as a no-op.
///
/// This protects config ergonomics so explicit `true` does not alter default behavior.
#[ntex::test]
async fn test_otlp_metric_attribute_true_override_is_noop() {
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
                  interval: 30ms
                  max_export_timeout: 50ms
              instrumentation:
                instruments:
                  http.server.request.duration:
                    attributes:
                      http.route: true
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

    wait_for_metrics_export().await;

    let metrics = otlp_collector.metrics_view().await;

    assert!(
        metrics.has_histogram(names::HTTP_SERVER_REQUEST_DURATION, &[]),
        "Expected {} histogram to stay enabled",
        names::HTTP_SERVER_REQUEST_DURATION
    );

    let attrs = metrics.latest_attribute_names(names::HTTP_SERVER_REQUEST_DURATION);
    assert!(
        attrs.contains(labels::HTTP_ROUTE),
        "Expected {} attribute to remain enabled",
        labels::HTTP_ROUTE
    );

    assert_metric_has_attrs(
        &metrics,
        names::HTTP_SERVER_REQUEST_DURATION,
        &[labels::ERROR_TYPE],
    );

    app.hold_until_shutdown(Box::new(otlp_collector));
}

/// Ensures parse failures increment GraphQL error counters and set server GraphQL error status.
///
/// This test focuses on GraphQL-layer semantics (`graphql.response.status`) rather than HTTP error
/// classification, which is covered by dedicated HTTP bad-request assertions.
#[ntex::test]
async fn test_otlp_graphql_errors_total_for_parsing_error() {
    let supergraph_path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("supergraph.graphql");

    let otlp_collector = OtlpCollector::start()
        .await
        .expect("Failed to start OTLP collector");
    let otlp_endpoint = otlp_collector.http_metrics_endpoint();

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
                  interval: 30ms
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

    let req = init_graphql_request("{ users { id }", None);
    test::call_service(&app.app, req.to_request()).await;

    let attrs = [(labels::CODE, "GRAPHQL_PARSE_FAILED")];
    wait_for_metrics_export().await;

    let metrics = otlp_collector.metrics_view().await;
    assert!(
        metrics.has_counter(names::GRAPHQL_ERRORS_TOTAL, &attrs),
        "Expected {} with code=GRAPHQL_PARSE_FAILED to exist",
        names::GRAPHQL_ERRORS_TOTAL
    );
    assert_counter_eq(&metrics, names::GRAPHQL_ERRORS_TOTAL, &attrs, 1.0);

    // Parse failures must be visible as GraphQL-level errors in server HTTP metrics.
    let http_attrs = [
        (labels::HTTP_REQUEST_METHOD, "POST"),
        (labels::HTTP_ROUTE, "/graphql"),
        (
            labels::GRAPHQL_RESPONSE_STATUS,
            values::GraphQLResponseStatus::Error.as_str(),
        ),
    ];
    assert_histogram_count(
        &metrics,
        names::HTTP_SERVER_REQUEST_DURATION,
        &http_attrs,
        1,
    );

    app.hold_until_shutdown(Box::new(otlp_collector));
}

/// Ensures malformed request bodies map to server-side HTTP error labeling.
///
/// This guards that `graphql.response.status=error` and `error.type=400` are emitted together.
#[ntex::test]
async fn test_otlp_http_server_bad_request_sets_graphql_status_and_error_type() {
    let supergraph_path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("supergraph.graphql");

    let otlp_collector = OtlpCollector::start()
        .await
        .expect("Failed to start OTLP collector");
    let otlp_endpoint = otlp_collector.http_metrics_endpoint();

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
                  interval: 30ms
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

    let req = test::TestRequest::post()
        .uri("/graphql")
        .header("content-type", "application/json")
        .set_payload("{")
        .to_request();
    test::call_service(&app.app, req).await;

    wait_for_metrics_export().await;

    let metrics = otlp_collector.metrics_view().await;
    let attrs = [
        (labels::HTTP_REQUEST_METHOD, "POST"),
        (labels::HTTP_ROUTE, "/graphql"),
        (labels::HTTP_RESPONSE_STATUS_CODE, "400"),
        (labels::ERROR_TYPE, "400"),
        (
            labels::GRAPHQL_RESPONSE_STATUS,
            values::GraphQLResponseStatus::Error.as_str(),
        ),
    ];

    assert_histogram_count(&metrics, names::HTTP_SERVER_REQUEST_DURATION, &attrs, 1);

    app.hold_until_shutdown(Box::new(otlp_collector));
}

/// Ensures subgraph transport failures are labeled as client GraphQL errors.
///
/// This verifies `graphql.response.status=error`, transport `error.type`, and no
/// `http.response.status_code` when no HTTP response was received from the subgraph.
#[ntex::test]
async fn test_otlp_http_client_transport_failure_sets_graphql_status_and_error_type() {
    let supergraph_path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("supergraph.graphql");

    let otlp_collector = OtlpCollector::start()
        .await
        .expect("Failed to start OTLP collector");
    let otlp_endpoint = otlp_collector.http_metrics_endpoint();

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
                  interval: 30ms
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

    wait_for_metrics_export().await;

    let metrics = otlp_collector.metrics_view().await;

    let attrs_request_failure = [
        (labels::HTTP_REQUEST_METHOD, "POST"),
        (
            labels::GRAPHQL_RESPONSE_STATUS,
            values::GraphQLResponseStatus::Error.as_str(),
        ),
        (labels::ERROR_TYPE, "SUBGRAPH_REQUEST_FAILURE"),
    ];

    assert_histogram_count(
        &metrics,
        names::HTTP_CLIENT_REQUEST_DURATION,
        &attrs_request_failure,
        1,
    );

    let attrs = metrics.latest_attribute_names(names::HTTP_CLIENT_REQUEST_DURATION);
    assert!(
        attrs.contains(labels::GRAPHQL_RESPONSE_STATUS),
        "Expected {} attribute to be present",
        labels::GRAPHQL_RESPONSE_STATUS
    );
    assert!(
        attrs.contains(labels::ERROR_TYPE),
        "Expected {} attribute to be present",
        labels::ERROR_TYPE
    );
    // Transport failures happen before receiving a valid HTTP response from the subgraph,
    // so status code must not be present on these series.
    assert!(
        !attrs.contains(labels::HTTP_RESPONSE_STATUS_CODE),
        "Expected {} to be absent on transport failure",
        labels::HTTP_RESPONSE_STATUS_CODE
    );

    app.hold_until_shutdown(Box::new(otlp_collector));
}
