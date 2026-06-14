pub use futures::StreamExt;
pub use std::time::Duration;

pub use sonic_rs::{json, JsonContainerTrait, JsonValueTrait};

pub use crate::testkit::{
    otel::{CollectedMetrics, OtlpCollector},
    ClientResponseExt, TestRouter, TestSubgraphs,
};
pub use hive_router_internal::telemetry::metrics::catalog::{labels, names};
pub use hive_router_plan_executor::executors::{
    graphql_transport_ws::SubscribePayload, websocket_client::WsClient,
};

pub(super) async fn wait_for_metrics_export() {
    tokio::time::sleep(Duration::from_millis(300)).await;
}

pub(super) fn assert_histogram_sample_count(
    metrics: &CollectedMetrics,
    name: &str,
    attrs: &[(&str, &str)],
    expected_count: u64,
) {
    let (count, _) = metrics.latest_histogram_count_sum(name, attrs);
    assert_eq!(
        count, expected_count,
        "Expected {name} sample count to be {expected_count}, got {count}"
    );
}

pub(super) fn assert_histogram_sample_count_at_least(
    metrics: &CollectedMetrics,
    name: &str,
    attrs: &[(&str, &str)],
    expected_min_count: u64,
) {
    let (count, _) = metrics.latest_histogram_count_sum(name, attrs);
    assert!(
        count >= expected_min_count,
        "Expected {name} sample count to be >= {expected_min_count}, got {count}"
    );
}

pub(super) async fn assert_estimated_too_expensive(
    query: &str,
    variables: Option<sonic_rs::Value>,
    expected_cost: u64,
) {
    let subgraphs = TestSubgraphs::builder().build().start().await;
    let router = TestRouter::builder()
        .with_subgraphs(&subgraphs)
        .inline_config(format!(
            r#"
            supergraph:
                    source: file
                    path: supergraph_demand_control.graphql
            demand_control:
                    enabled: true
                    mode: enforce
                    strategy:
                      static_estimated:
                        max: {}
            "#,
            expected_cost.saturating_sub(1)
        ))
        .build()
        .start()
        .await;

    let res = router.send_graphql_request(query, variables, None).await;
    let json = res.json_body().await;

    assert_eq!(
        json["errors"][0]["extensions"]["code"].as_str(),
        Some("COST_ESTIMATED_TOO_EXPENSIVE")
    );
    assert_eq!(
        json["errors"][0]["message"].as_str(),
        Some("Operation estimated cost exceeds max cost")
    );
}

// No directives/custom list size: baseline query should estimate to 4.
