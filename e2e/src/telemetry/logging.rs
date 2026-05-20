use crate::testkit::{
    otel::OtlpCollector,
    stdout::{CaptureStdoutExt, StdOutCaptureBridge},
    Started, TestRouter, TestRouterBuilder, TestSubgraphs,
};
use hive_router_internal::logging::context::{
    LOG_GRAPHQL_REQUEST_COMPLETED, LOG_GRAPHQL_REQUEST_START, LOG_HTTP_REQUEST_COMPLETED,
    LOG_HTTP_REQUEST_START, LOG_SUBGRAPH_REQUEST_COMPLETED, LOG_SUBGRAPH_REQUEST_START,
};
use http::{HeaderMap, HeaderName, HeaderValue};
use insta::assert_json_snapshot;
use serde_json::{Map, Value};

const TEST_QUERY: &str = "{ users { id } }";
const TRACEPARENT_VALUE: &str = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";
const TRACEPARENT_TRACE_ID: &str = "4bf92f3577b34da6a3ce929d0e0e4736";
const LOG_LINES_BASELINE: usize = 6;

/// Builds a router config combining the standard supergraph block with the given telemetry YAML.
/// `telemetry_yaml` must start with `telemetry:` at column 0.
fn router_with_telemetry(telemetry_yaml: &str) -> TestRouterBuilder {
    let supergraph_path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("supergraph.graphql");
    TestRouter::builder().inline_config(format!(
        "supergraph:\n  source: file\n  path: {}\n\n{}",
        supergraph_path.to_str().unwrap(),
        telemetry_yaml,
    ))
}

/// Builds a router with telemetry logging exporting to stdout in the given format ("text" or "json").
fn router_with_stdout_logging(format: &str) -> TestRouterBuilder {
    router_with_telemetry(&format!(
        "\
telemetry:
  logging:
    service:
      exporters:
        - kind: stdout
          level: info
          format: {format}
"
    ))
}

/// Starts subgraphs and a router wired together. The subgraphs must remain alive for the
/// duration of the test, so the caller should bind them to a `_subgraphs` variable.
async fn setup_router(builder: TestRouterBuilder) -> (TestSubgraphs<Started>, TestRouter<Started>) {
    let subgraphs = TestSubgraphs::builder().build().start().await;
    let router = builder.with_subgraphs(&subgraphs).build().start().await;
    (subgraphs, router)
}

fn single_header(name: &'static str, value: &'static str) -> HeaderMap {
    HeaderMap::from_iter([(
        HeaderName::from_static(name),
        HeaderValue::from_static(value),
    )])
}

fn span_attr<'a>(log_line: &'a Map<String, Value>, attr: &str) -> Option<&'a Value> {
    log_line
        .get("span")
        .and_then(Value::as_object)
        .and_then(|s| s.get(attr))
}

fn span_str_attr(log_line: &Map<String, Value>, attr: &str) -> String {
    span_attr(log_line, attr)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("missing/invalid span attribute: {attr}"))
        .to_string()
}

fn req_id_of(log_line: &Map<String, Value>) -> String {
    span_str_attr(log_line, "req_id")
}

fn log_line<'a>(capture: &'a StdOutCaptureBridge, msg: &str) -> &'a Map<String, Value> {
    capture
        .by_message(msg)
        .unwrap_or_else(|| panic!("missing log line: {msg}"))
}

/// This test is a bit dumb, but the goal is to confirm that just running the logging setup without OTEL enabled
/// does not cause OTEL to kick in.
/// Also, it makes sure that enabling OTEL doesn't cause the spans to become over noisy.
#[ntex::test]
async fn test_log_contents_with_just_logging_enabled() {
    let (_subgraphs, router) = setup_router(router_with_stdout_logging("text")).await;

    let stdout_log = router
        .send_graphql_request(TEST_QUERY, None, None)
        .capture_stdout_lines()
        .await;

    assert_eq!(stdout_log.len(), LOG_LINES_BASELINE);
}

#[ntex::test]
async fn test_log_contents_with_otel_also_enabled() {
    let otlp_collector = OtlpCollector::start()
        .await
        .expect("Failed to start OTLP collector");
    let _insta_settings_guard = otlp_collector.insta_filter_settings().bind_to_scope();
    let otlp_endpoint = otlp_collector.http_traces_endpoint();

    let (_subgraphs, router) = setup_router(router_with_telemetry(&format!(
        "\
telemetry:
  tracing:
    exporters:
      - kind: otlp
        endpoint: {otlp_endpoint}
        protocol: http
        batch_processor:
          scheduled_delay: 50ms
          max_export_timeout: 2s
  logging:
    service:
      exporters:
        - kind: stdout
          level: info
"
    )))
    .await;

    let stdout_log = router
        .send_graphql_request(TEST_QUERY, None, None)
        .capture_stdout_lines()
        .await;

    assert_eq!(stdout_log.len(), LOG_LINES_BASELINE);
}

#[ntex::test]
async fn should_output_json_correctly() {
    let (_subgraphs, router) = setup_router(router_with_stdout_logging("json")).await;

    // Just checking for existence here, if the messages got parsed correctly, then we are all good in terms of json structure
    let _stdout_log = router
        .send_graphql_request(TEST_QUERY, None, None)
        .capture_stdout_json()
        .await;
}

#[ntex::test]
async fn request_identifiers_detection() {
    let (_subgraphs, router) = setup_router(router_with_stdout_logging("json")).await;

    let attr_from_request = async |attr: &str, headers: HeaderMap| -> String {
        let stdout_log = router
            .send_graphql_request(TEST_QUERY, None, Some(headers))
            .capture_stdout_json()
            .await;
        span_str_attr(log_line(&stdout_log, LOG_HTTP_REQUEST_START), attr)
    };

    // No request ID specified, use a generated one to make request logs identifiable
    let req_id = attr_from_request("req_id", HeaderMap::new()).await;
    assert!(!req_id.is_empty());

    // Request ID specified, verify it appears in the logs
    let req_id = attr_from_request("req_id", single_header("x-request-id", "abc")).await;
    assert_eq!(req_id, "abc");

    // Trace ID specified via traceparent
    let trace_id =
        attr_from_request("trace_id", single_header("traceparent", TRACEPARENT_VALUE)).await;
    assert_eq!(trace_id, TRACEPARENT_TRACE_ID);
}

#[ntex::test]
async fn request_id_correlated_in_nested_log_lines() {
    let (_subgraphs, router) = setup_router(router_with_stdout_logging("json")).await;

    let stdout_log = router
        .send_graphql_request(TEST_QUERY, None, None)
        .capture_stdout_json()
        .await;

    let http_start = log_line(&stdout_log, LOG_HTTP_REQUEST_START);
    let http_end = log_line(&stdout_log, LOG_HTTP_REQUEST_COMPLETED);
    let gql_start = log_line(&stdout_log, LOG_GRAPHQL_REQUEST_START);
    let gql_end = log_line(&stdout_log, LOG_GRAPHQL_REQUEST_COMPLETED);
    let subgraph_start = log_line(&stdout_log, LOG_SUBGRAPH_REQUEST_START);
    let subgraph_end = log_line(&stdout_log, LOG_SUBGRAPH_REQUEST_COMPLETED);

    assert_eq!(req_id_of(http_start), req_id_of(http_end));
    assert_eq!(req_id_of(http_start), req_id_of(gql_start));
    assert_eq!(req_id_of(http_start), req_id_of(subgraph_start));
    assert_eq!(req_id_of(gql_start), req_id_of(gql_end));
    assert_eq!(req_id_of(subgraph_start), req_id_of(subgraph_end));
}

#[ntex::test]
async fn json_log_default_attrs() {
    let (_subgraphs, router) = setup_router(router_with_stdout_logging("json")).await;

    let stdout_log = router
        .send_graphql_request(TEST_QUERY, None, None)
        .capture_stdout_json()
        .await;

    let http_start = log_line(&stdout_log, LOG_HTTP_REQUEST_START);
    let http_end = log_line(&stdout_log, LOG_HTTP_REQUEST_COMPLETED);
    let gql_start = log_line(&stdout_log, LOG_GRAPHQL_REQUEST_START);
    let gql_end = log_line(&stdout_log, LOG_GRAPHQL_REQUEST_COMPLETED);
    let subgraph_start = log_line(&stdout_log, LOG_SUBGRAPH_REQUEST_START);
    let subgraph_end = log_line(&stdout_log, LOG_SUBGRAPH_REQUEST_COMPLETED);

    assert_json_snapshot!(http_start, {
      ".timestamp" => "[timestamp]",
      ".span.req_id" => "[req_id]",
    });

    assert_json_snapshot!(http_end, {
      ".duration_ms" => "[duration_ms]",
      ".span.req_id" => "[req_id]",
      ".timestamp" => "[timestamp]",
    });

    assert_json_snapshot!(gql_start, {
      ".timestamp" => "[timestamp]",
      ".span.req_id" => "[req_id]",
    });

    assert_json_snapshot!(gql_end, {
      ".timestamp" => "[timestamp]",
      ".span.req_id" => "[req_id]",
    });

    assert_json_snapshot!(subgraph_start, {
      ".timestamp" => "[timestamp]",
      ".span.req_id" => "[req_id]",
    });

    assert_json_snapshot!(subgraph_end, {
      ".timestamp" => "[timestamp]",
      ".span.req_id" => "[req_id]",
    });
}

#[ntex::test]
async fn should_allow_to_customize_req_id_header_name() {
    let (_subgraphs, router) = setup_router(router_with_telemetry(
        "\
telemetry:
  logging:
    service:
      correlation:
        id_header: x-ray-id
      exporters:
        - kind: stdout
          level: info
          format: json
",
    ))
    .await;

    let stdout_log = router
        .send_graphql_request(TEST_QUERY, None, Some(single_header("x-request-id", "123")))
        .capture_stdout_json()
        .await;
    assert_ne!(
        req_id_of(log_line(&stdout_log, LOG_HTTP_REQUEST_START)),
        "123"
    );

    let stdout_log = router
        .send_graphql_request(TEST_QUERY, None, Some(single_header("x-ray-id", "123")))
        .capture_stdout_json()
        .await;
    assert_eq!(
        req_id_of(log_line(&stdout_log, LOG_HTTP_REQUEST_START)),
        "123"
    );
}

#[ntex::test]
async fn should_allow_to_disable_trace_id() {
    let (_subgraphs, router) = setup_router(router_with_telemetry(
        "\
telemetry:
  logging:
    service:
      correlation:
        trace_propagation: false
      exporters:
        - kind: stdout
          level: info
          format: json
",
    ))
    .await;

    let stdout_log = router
        .send_graphql_request(
            TEST_QUERY,
            None,
            Some(single_header("traceparent", TRACEPARENT_VALUE)),
        )
        .capture_stdout_json()
        .await;
    let http_start = log_line(&stdout_log, LOG_HTTP_REQUEST_START);
    assert!(span_attr(http_start, "trace_id").is_none());
}

#[ntex::test]
async fn should_allow_to_customize_http_fields() {
    let (_subgraphs, router) = setup_router(router_with_telemetry(
        "\
telemetry:
  logging:
    service:
      log_fields:
        http:
          request:
            method: false
            query_string: true
            headers:
              - x-custom-foo
              - x-custom-bar
          response:
            status_code: false
            headers:
              - content-type
            payload_bytes: true
      exporters:
        - kind: stdout
          level: info
          format: json
",
    ))
    .await;

    let stdout_log = router
        .send_graphql_request(
            TEST_QUERY,
            None,
            Some(single_header("x-custom-foo", "hello")),
        )
        .capture_stdout_json()
        .await;

    let http_start = log_line(&stdout_log, LOG_HTTP_REQUEST_START);
    assert!(
        http_start.get("method").is_none(),
        "method should be omitted when method: false"
    );
    assert!(
        http_start.contains_key("query_string"),
        "query_string should be emitted when query_string: true"
    );
    let request_headers = http_start
        .get("headers")
        .and_then(Value::as_object)
        .expect("request headers should be an object");
    assert_eq!(
        request_headers.get("x-custom-foo").and_then(Value::as_str),
        Some("hello"),
    );
    assert!(
        request_headers
            .get("x-custom-bar")
            .is_some_and(Value::is_null),
        "configured but unsent header should be null"
    );
    assert!(
        !request_headers.contains_key("accept") && !request_headers.contains_key("user-agent"),
        "default headers should not appear when a custom list is configured"
    );

    let http_end = log_line(&stdout_log, LOG_HTTP_REQUEST_COMPLETED);
    assert!(
        http_end.get("status_code").is_none(),
        "status_code should be omitted when status_code: false"
    );
    let response_headers = http_end
        .get("headers")
        .and_then(Value::as_object)
        .expect("response headers should be an object");
    assert!(
        response_headers.contains_key("content-type"),
        "configured response header should be present in the log"
    );
    let payload_bytes = http_end
        .get("payload_bytes")
        .and_then(Value::as_i64)
        .expect("payload_bytes should be an integer when payload_bytes: true");
    assert!(
        payload_bytes > 0,
        "payload_bytes should be > 0 for a non-empty response, got {payload_bytes}"
    );
}

#[ntex::test]
async fn should_allow_to_customize_graphql_fields() {
    let (_subgraphs, router) = setup_router(router_with_telemetry(
        "\
telemetry:
  logging:
    service:
      log_fields:
        graphql:
          request:
            body_size_bytes: true
            operation: true
            variables: true
          response:
            error_count: true
      exporters:
        - kind: stdout
          level: info
          format: json
",
    ))
    .await;

    let query = "query($id: ID!) { user(id: $id) { id } }";
    let stdout_log = router
        .send_graphql_request(query, Some(sonic_rs::json!({ "id": "1" })), None)
        .capture_stdout_json()
        .await;

    let gql_start = log_line(&stdout_log, LOG_GRAPHQL_REQUEST_START);
    let body_size_bytes = gql_start
        .get("body_size_bytes")
        .and_then(Value::as_i64)
        .expect("body_size_bytes should be an integer when body_size_bytes: true");
    assert!(
        body_size_bytes > 0,
        "body_size_bytes should be > 0, got {body_size_bytes}"
    );
    assert_eq!(
        gql_start.get("operation").and_then(Value::as_str),
        Some(query),
        "operation should match the sent query when operation: true",
    );
    let variables = gql_start
        .get("variables")
        .and_then(Value::as_object)
        .expect("variables should be an object when variables: true");
    assert_eq!(
        variables.get("id").and_then(Value::as_str),
        Some("1"),
        "variables.id should match the sent value"
    );

    let gql_end = log_line(&stdout_log, LOG_GRAPHQL_REQUEST_COMPLETED);
    assert_eq!(
        gql_end.get("error_count").and_then(Value::as_i64),
        Some(0),
        "error_count should be 0 for a successful request when error_count: true",
    );
}

#[ntex::test]
async fn should_allow_to_customize_subgraph_fields() {
    let (_subgraphs, router) = setup_router(router_with_telemetry(
        "\
telemetry:
  logging:
    service:
      log_fields:
        subgraph:
          request:
            operation: false
            operation_name: true
            variables: true
          response:
            error_count: false
      exporters:
        - kind: stdout
          level: info
          format: json
",
    ))
    .await;

    let query = "query GetUser($id: ID!) { user(id: $id) { id } }";
    let stdout_log = router
        .send_graphql_request(query, Some(sonic_rs::json!({ "id": "1" })), None)
        .capture_stdout_json()
        .await;

    let subgraph_start = log_line(&stdout_log, LOG_SUBGRAPH_REQUEST_START);
    assert!(
        subgraph_start.get("operation").is_none(),
        "operation should be omitted when operation: false"
    );
    assert!(
        subgraph_start.contains_key("operation_name"),
        "operation_name should be emitted when operation_name: true"
    );
    let variables = subgraph_start
        .get("variables")
        .and_then(Value::as_object)
        .expect("variables should be an object when variables: true");
    assert_eq!(
        variables.get("id").and_then(Value::as_str),
        Some("1"),
        "variables.id should match the sent value"
    );

    let subgraph_end = log_line(&stdout_log, LOG_SUBGRAPH_REQUEST_COMPLETED);
    assert!(
        subgraph_end.get("error_count").is_none(),
        "error_count should be omitted when error_count: false"
    );
}
