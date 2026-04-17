use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;

use crate::testkit::{TestRouter, TestSubgraphs};

/// A simple mock HTTP server that captures usage report requests.
struct MockUsageEndpoint {
    address: String,
    reports: Arc<Mutex<Vec<serde_json::Value>>>,
    _handle: std::thread::JoinHandle<()>,
}

impl MockUsageEndpoint {
    fn start() -> Self {
        let server =
            tiny_http::Server::http("127.0.0.1:0").expect("Failed to start mock usage endpoint");
        let address = format!("http://{}", server.server_addr());
        let reports: Arc<Mutex<Vec<serde_json::Value>>> = Arc::new(Mutex::new(Vec::new()));
        let reports_clone = reports.clone();

        let handle = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("Failed to create tokio runtime for mock usage endpoint");

            for mut request in server.incoming_requests() {
                let mut body = Vec::new();
                let _ = request.as_reader().read_to_end(&mut body);

                if let Ok(json) = serde_json::from_slice::<serde_json::Value>(&body) {
                    rt.block_on(async {
                        reports_clone.lock().await.push(json);
                    });
                }

                let response = tiny_http::Response::from_string("{}");
                let _ = request.respond(response);
            }
        });

        MockUsageEndpoint {
            address,
            reports,
            _handle: handle,
        }
    }

    /// Wait until at least `count` reports have been received, with a timeout.
    async fn wait_for_reports(&self, count: usize) {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if self.reports.lock().await.len() >= count {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .unwrap_or_else(|_| {
            panic!(
                "Timed out waiting for {} usage reports, got {}",
                count,
                self.reports.try_lock().map(|r| r.len()).unwrap_or(0)
            )
        });
    }

    /// Wait for a short duration and assert no reports arrived.
    async fn assert_no_reports(&self, wait: Duration) {
        tokio::time::sleep(wait).await;
        let reports = self.reports.lock().await;
        assert!(
            reports.is_empty(),
            "Expected no usage reports but got {}",
            reports.len()
        );
    }

    async fn reports(&self) -> Vec<serde_json::Value> {
        self.reports.lock().await.clone()
    }

    /// Returns the total number of operations across all received reports.
    async fn total_operations_count(&self) -> usize {
        let reports = self.reports.lock().await;
        reports
            .iter()
            .filter_map(|r| r.get("operations"))
            .filter_map(|ops| ops.as_array())
            .map(|ops| ops.len())
            .sum()
    }
}

/// Test that reports are sent when no exclude expression is configured.
#[ntex::test]
async fn usage_reporting_sends_reports_without_exclude() {
    let supergraph_path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("supergraph.graphql");
    let supergraph_path = supergraph_path.to_str().unwrap();

    let mock = MockUsageEndpoint::start();
    let usage_endpoint = &mock.address;

    let subgraphs = TestSubgraphs::builder().build().start().await;

    let router = TestRouter::builder()
        .inline_config(format!(
            r#"
            supergraph:
              source: file
              path: {supergraph_path}

            telemetry:
              hive:
                token: test-token
                usage_reporting:
                  enabled: true
                  endpoint: {usage_endpoint}
                  buffer_size: 1
                  flush_interval: 100ms
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

    // The buffer_size=1 means flush happens immediately after the first report
    mock.wait_for_reports(1).await;

    let reports = mock.reports().await;
    assert!(!reports.is_empty(), "Expected at least one report");
    let first_report = &reports[0];
    assert!(
        first_report.get("operations").is_some(),
        "Report should contain operations"
    );

    drop(router);
}

/// Test that an exclude expression matching the operation name prevents the report from being sent.
#[ntex::test]
async fn usage_reporting_excludes_by_operation_name() {
    let supergraph_path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("supergraph.graphql");
    let supergraph_path = supergraph_path.to_str().unwrap();

    let mock = MockUsageEndpoint::start();
    let usage_endpoint = &mock.address;

    let subgraphs = TestSubgraphs::builder().build().start().await;

    let router = TestRouter::builder()
        .inline_config(format!(
            r#"
            supergraph:
              source: file
              path: {supergraph_path}

            telemetry:
              hive:
                token: test-token
                usage_reporting:
                  enabled: true
                  endpoint: {usage_endpoint}
                  buffer_size: 1
                  flush_interval: 100ms
                  exclude: '.request.operation.name == "ExcludedOp"'
            "#,
        ))
        .with_subgraphs(&subgraphs)
        .build()
        .start()
        .await;

    // Send a request with the excluded operation name
    let res = router
        .send_graphql_request("query ExcludedOp { users { id } }", None, None)
        .await;
    assert!(res.status().is_success());

    // Wait a reasonable time to ensure no reports arrive
    mock.assert_no_reports(Duration::from_secs(2)).await;

    drop(router);
}

/// Test that an exclude expression does NOT exclude non-matching operations.
#[ntex::test]
async fn usage_reporting_does_not_exclude_non_matching_operations() {
    let supergraph_path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("supergraph.graphql");
    let supergraph_path = supergraph_path.to_str().unwrap();

    let mock = MockUsageEndpoint::start();
    let usage_endpoint = &mock.address;

    let subgraphs = TestSubgraphs::builder().build().start().await;

    let router = TestRouter::builder()
        .inline_config(format!(
            r#"
            supergraph:
              source: file
              path: {supergraph_path}

            telemetry:
              hive:
                token: test-token
                usage_reporting:
                  enabled: true
                  endpoint: {usage_endpoint}
                  buffer_size: 1
                  flush_interval: 100ms
                  exclude: '.request.operation.name == "ExcludedOp"'
            "#,
        ))
        .with_subgraphs(&subgraphs)
        .build()
        .start()
        .await;

    // Send a request with a different operation name (should NOT be excluded)
    let res = router
        .send_graphql_request("query AllowedOp { users { id } }", None, None)
        .await;
    assert!(res.status().is_success());

    mock.wait_for_reports(1).await;

    let reports = mock.reports().await;
    assert!(
        !reports.is_empty(),
        "Expected report for non-excluded operation"
    );

    drop(router);
}

/// Test that an exclude expression can filter by request header.
#[ntex::test]
async fn usage_reporting_excludes_by_header() {
    let supergraph_path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("supergraph.graphql");
    let supergraph_path = supergraph_path.to_str().unwrap();

    let mock = MockUsageEndpoint::start();
    let usage_endpoint = &mock.address;

    let subgraphs = TestSubgraphs::builder().build().start().await;

    let router = TestRouter::builder()
        .inline_config(format!(
            r#"
            supergraph:
              source: file
              path: {supergraph_path}

            telemetry:
              hive:
                token: test-token
                usage_reporting:
                  enabled: true
                  endpoint: {usage_endpoint}
                  buffer_size: 1
                  flush_interval: 100ms
                  exclude: '.request.headers."x-internal" == "true"'
            "#,
        ))
        .with_subgraphs(&subgraphs)
        .build()
        .start()
        .await;

    // Send with the x-internal header - should be excluded
    let res = router
        .send_graphql_request(
            "{ users { id } }",
            None,
            Some({
                let mut headers = http::HeaderMap::new();
                headers.insert("x-internal", "true".parse().unwrap());
                headers
            }),
        )
        .await;
    assert!(res.status().is_success());

    mock.assert_no_reports(Duration::from_secs(2)).await;

    drop(router);
}

/// Test that requests without the excluded header still get reported.
#[ntex::test]
async fn usage_reporting_sends_when_header_not_matching() {
    let supergraph_path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("supergraph.graphql");
    let supergraph_path = supergraph_path.to_str().unwrap();

    let mock = MockUsageEndpoint::start();
    let usage_endpoint = &mock.address;

    let subgraphs = TestSubgraphs::builder().build().start().await;

    let router = TestRouter::builder()
        .inline_config(format!(
            r#"
            supergraph:
              source: file
              path: {supergraph_path}

            telemetry:
              hive:
                token: test-token
                usage_reporting:
                  enabled: true
                  endpoint: {usage_endpoint}
                  buffer_size: 1
                  flush_interval: 100ms
                  exclude: '.request.headers."x-internal" == "true"'
            "#,
        ))
        .with_subgraphs(&subgraphs)
        .build()
        .start()
        .await;

    // Send without the x-internal header - should NOT be excluded
    let res = router
        .send_graphql_request("{ users { id } }", None, None)
        .await;
    assert!(res.status().is_success());

    mock.wait_for_reports(1).await;

    let reports = mock.reports().await;
    assert!(
        !reports.is_empty(),
        "Expected report when header does not match"
    );

    drop(router);
}

/// Test a complex exclude expression using if/else to exclude introspection AND mutations.
#[ntex::test]
async fn usage_reporting_complex_exclude_expression() {
    let supergraph_path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("supergraph.graphql");
    let supergraph_path = supergraph_path.to_str().unwrap();

    let mock_excluded = MockUsageEndpoint::start();
    let usage_endpoint = &mock_excluded.address;

    let subgraphs = TestSubgraphs::builder().build().start().await;

    // Exclude IntrospectionQuery by name OR any request with x-exclude header
    let exclude_expr = r#"if (.request.operation.name == "IntrospectionQuery") { true } else if (.request.headers."x-exclude" == "yes") { true } else { false }"#;

    let router = TestRouter::builder()
        .inline_config(format!(
            r#"
            supergraph:
              source: file
              path: {supergraph_path}

            telemetry:
              hive:
                token: test-token
                usage_reporting:
                  enabled: true
                  endpoint: {usage_endpoint}
                  buffer_size: 1
                  flush_interval: 100ms
                  exclude: '{exclude_expr}'
            "#,
        ))
        .with_subgraphs(&subgraphs)
        .build()
        .start()
        .await;

    // 1st request: IntrospectionQuery - should be excluded
    let res = router
        .send_graphql_request("query IntrospectionQuery { __typename }", None, None)
        .await;
    assert!(res.status().is_success());

    // 2nd request: with x-exclude header - should be excluded
    let res = router
        .send_graphql_request(
            "query SomeOp { users { id } }",
            None,
            Some({
                let mut headers = http::HeaderMap::new();
                headers.insert("x-exclude", "yes".parse().unwrap());
                headers
            }),
        )
        .await;
    assert!(res.status().is_success());

    // Neither should have generated a report
    mock_excluded
        .assert_no_reports(Duration::from_secs(2))
        .await;

    // 3rd request: normal query - should be reported
    let res = router
        .send_graphql_request("query NormalQuery { users { id } }", None, None)
        .await;
    assert!(res.status().is_success());

    mock_excluded.wait_for_reports(1).await;

    let total_ops = mock_excluded.total_operations_count().await;
    assert!(
        total_ops >= 1,
        "Expected at least one operation in reports for non-excluded query"
    );

    drop(router);
}
