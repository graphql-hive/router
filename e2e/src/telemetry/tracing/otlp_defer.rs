use std::time::Duration;

use futures::StreamExt;

use crate::testkit::{
    otel::{spans_by_hive_kind as by_kind, CollectedSpan, OtlpCollector},
    some_header_map, ClientResponseExt, Started, TestRouter, TestSubgraphs,
};

/// The deferred `inStock` comes from `inventory`, its fetch starts after the primary part is out.
const QUERY: &str = "{ topProducts(first: 1) { upc ... @defer { inStock } } }";

/// A router with `@defer` on that sends its traces to `otlp_collector`, `inventory` answers
/// after `inventory_delay`.
async fn start(
    otlp_collector: &OtlpCollector,
    inventory_delay: Duration,
) -> (TestSubgraphs<Started>, TestRouter<Started>) {
    let supergraph_path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("supergraph.graphql");
    let supergraph_path = supergraph_path.to_str().unwrap();
    let otlp_endpoint = otlp_collector.http_traces_endpoint();

    let subgraphs = TestSubgraphs::builder()
        .with_path_delay("/inventory", inventory_delay)
        .build()
        .start()
        .await;

    let router = TestRouter::builder()
        .inline_config(format!(
            r#"
          supergraph:
            source: file
            path: {supergraph_path}

          defer:
            enabled: true

          telemetry:
            tracing:
              exporters:
                - kind: otlp
                  endpoint: {otlp_endpoint}
                  protocol: http
                  batch_processor:
                    scheduled_delay: 50ms
                    max_export_timeout: 2s
      "#,
        ))
        .with_subgraphs(&subgraphs)
        .build()
        .start()
        .await;

    (subgraphs, router)
}

fn attribute<'a>(span: &'a CollectedSpan, name: &str) -> Option<&'a str> {
    span.attributes.get(name).map(String::as_str)
}

/// A deferred response keeps its trace together: the deferred fetch runs after the primary part
/// is out, and its span still lands under `graphql.execute`, in the same trace. The spans above
/// it stay open until the last part, and `http.server` says how the response ended.
#[ntex::test]
async fn test_otlp_deferred_fetches_stay_in_the_trace() {
    let otlp_collector = OtlpCollector::start()
        .await
        .expect("Failed to start OTLP collector");
    let (_subgraphs, router) = start(&otlp_collector, Duration::from_millis(300)).await;

    let response = router
        .send_graphql_request(
            QUERY,
            None,
            some_header_map! { http::header::ACCEPT => "multipart/mixed" },
        )
        .await;
    assert_eq!(response.status(), 200);
    let body = response.string_body().await;
    assert!(body.contains(r#""hasNext":false"#), "{body}");

    let spans = otlp_collector
        .wait_for_spans(|spans| {
            by_kind(spans, "graphql.subgraph.operation").len() >= 2
                && !by_kind(spans, "http.server").is_empty()
        })
        .await;
    let http_server = by_kind(&spans, "http.server")[0];
    let operation = by_kind(&spans, "graphql.operation")[0];
    let execute = by_kind(&spans, "graphql.execute")[0];
    let subgraph = |name: &str| {
        *by_kind(&spans, "graphql.subgraph.operation")
            .iter()
            .find(|span| attribute(span, "hive.graphql.subgraph.name") == Some(name))
            .unwrap_or_else(|| panic!("no span for subgraph {name}"))
    };
    let products = subgraph("products");
    let inventory = subgraph("inventory");

    for span in [operation, execute, products, inventory] {
        assert_eq!(
            span.trace_id, http_server.trace_id,
            "{} should be in the request's trace",
            span.name
        );
    }
    assert_eq!(products.parent_span_id, execute.id);
    assert_eq!(
        inventory.parent_span_id, execute.id,
        "the deferred fetch should be under graphql.execute like the primary one"
    );

    for span in [execute, operation, http_server] {
        assert!(
            span.end_time_unix_nano >= inventory.end_time_unix_nano,
            "{} should stay open until the deferred fetch is done",
            span.name
        );
    }
    assert_eq!(
        attribute(http_server, "hive.defer.end_reason"),
        Some("completed")
    );
    assert_eq!(
        attribute(http_server, "http.response.body.size"),
        Some(body.len().to_string().as_str()),
        "the body size should count every part"
    );
}

/// A client that leaves after the primary part ends the response there: `http.server` ends
/// then, with `client_disconnect`, and doesn't wait for the slow deferred call.
#[ntex::test]
async fn test_otlp_deferred_response_records_a_client_disconnect() {
    let otlp_collector = OtlpCollector::start()
        .await
        .expect("Failed to start OTLP collector");
    let (_subgraphs, router) = start(&otlp_collector, Duration::from_millis(2000)).await;

    let mut response = router
        .send_graphql_request(
            QUERY,
            None,
            some_header_map! { http::header::ACCEPT => "multipart/mixed" },
        )
        .await;
    let primary = response.next().await.unwrap().unwrap();
    assert!(String::from_utf8_lossy(&primary).contains(r#""hasNext":true"#));
    drop(response);

    let spans = otlp_collector
        .wait_for_spans(|spans| !by_kind(spans, "http.server").is_empty())
        .await;
    let http_server = by_kind(&spans, "http.server")[0];
    assert_eq!(
        attribute(http_server, "hive.defer.end_reason"),
        Some("client_disconnect")
    );
    let duration =
        Duration::from_nanos(http_server.end_time_unix_nano - http_server.start_time_unix_nano);
    assert!(
        duration < Duration::from_millis(2000),
        "http.server should end when the client left, took {duration:?}"
    );
}
