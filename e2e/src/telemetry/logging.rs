use crate::{
    some_header_map,
    testkit::{
        otel::OtlpCollector,
        stdout::{CaptureStdoutExt, StdOutCaptureBridge},
        Started, TestRouter, TestRouterBuilder, TestSubgraphs,
    },
};
use hive_router_internal::telemetry::logging::targets;
use http::{HeaderMap, HeaderName, HeaderValue};
use insta::assert_json_snapshot;
use serde_json::{Map, Value};
use sonic_rs::json;

const TEST_QUERY: &str = "{ users { id } }";
const TRACEPARENT_VALUE: &str = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";
const TRACEPARENT_TRACE_ID: &str = "4bf92f3577b34da6a3ce929d0e0e4736";
const LOG_LINES_BASELINE: usize = 1;

const LOG_HTTP_REQUEST_START: &str = "http request started";
const LOG_HTTP_REQUEST_COMPLETED: &str = "http request completed";
const LOG_GRAPHQL_REQUEST_START: &str = "graphql request started";
const LOG_GRAPHQL_REQUEST_COMPLETED: &str = "graphql request completed";
const LOG_SUBGRAPH_REQUEST_START: &str = "executing subgraph request";
const LOG_SUBGRAPH_REQUEST_COMPLETED: &str = "subgraph execution completed";

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
fn router_with_stdout_logging(format: &str, level: &str) -> TestRouterBuilder {
    router_with_telemetry(&format!(
        "\
log:
  level: {level}
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

fn find_attr(log_line: &Map<String, Value>, attr: &str) -> Option<String> {
    log_line
        .get(attr)
        .and_then(Value::as_str)
        .map(|s| s.to_string())
}

fn req_id_of(log_line: &Map<String, Value>) -> Option<String> {
    find_attr(log_line, "request_id").map(|id| id.to_string())
}

fn log_line<'a>(capture: &'a StdOutCaptureBridge, msg: &str) -> &'a Map<String, Value> {
    capture
        .by_message(msg)
        .unwrap_or_else(|| panic!("missing log line: {msg}"))
}

fn log_line_by_target<'a>(
    capture: &'a StdOutCaptureBridge,
    target: &str,
) -> &'a Map<String, Value> {
    capture
        .lines_json
        .iter()
        .find(|v| v.get("target").is_some_and(|v| v == target))
        .unwrap_or_else(|| panic!("missing log line: {target}"))
}

/// This test is a bit dumb, but the goal is to confirm that just running the logging setup without OTEL enabled
/// does not cause OTEL to kick in.
/// Also, it makes sure that enabling OTEL doesn't cause the spans to become over noisy.
#[ntex::test]
async fn test_log_contents_with_just_logging_enabled() {
    let (_subgraphs, router) = setup_router(router_with_stdout_logging("text", "info")).await;

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
log:
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
    let (_subgraphs, router) = setup_router(router_with_stdout_logging("json", "info")).await;

    // Just checking for existence here, if the messages got parsed correctly, then we are all good in terms of json structure
    let _stdout_log = router
        .send_graphql_request(TEST_QUERY, None, None)
        .capture_stdout_json()
        .await;
}

#[ntex::test]
async fn request_identifiers_detection() {
    let (_subgraphs, router) = setup_router(router_with_stdout_logging("json", "debug")).await;

    let attr_from_request = async |attr: &str, headers: HeaderMap| -> Option<String> {
        let stdout_log = router
            .send_graphql_request(TEST_QUERY, None, Some(headers))
            .capture_stdout_json()
            .await;

        find_attr(log_line(&stdout_log, LOG_HTTP_REQUEST_START), attr)
    };

    // No request ID specified, use a generated one to make request logs identifiable
    let req_id = attr_from_request("request_id", HeaderMap::new()).await;
    assert!(!req_id.is_none());

    // Request ID specified, verify it appears in the logs
    let req_id = attr_from_request("request_id", single_header("x-request-id", "abc")).await;
    assert_eq!(req_id, Some("abc".to_string()));

    // Trace ID specified via traceparent
    let trace_id =
        attr_from_request("trace_id", single_header("traceparent", TRACEPARENT_VALUE)).await;
    assert_eq!(trace_id, Some(TRACEPARENT_TRACE_ID.to_string()));
}

#[ntex::test]
async fn request_id_correlated_in_nested_log_lines() {
    let (_subgraphs, router) = setup_router(router_with_stdout_logging("json", "debug")).await;

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
async fn json_log_req_summary() {
    let (_subgraphs, router) = setup_router(router_with_stdout_logging("json", "info")).await;

    // valid query
    let stdout_log = router
        .send_graphql_request(TEST_QUERY, None, None)
        .capture_stdout_json()
        .await;
    let req_summary = log_line_by_target(&stdout_log, targets::SUMMARY);
    assert_json_snapshot!(req_summary, {
      ".timestamp" => "[timestamp]",
      ".request_id" => "[req_id]",
      ".duration_ms" => "[duration_ms]"
    }, @r#"
    {
      "duration_ms": "[duration_ms]",
      "error_count": 0,
      "involved_subgraphs": "accounts",
      "level": "INFO",
      "operation_hash": "e92177e49c0010d4e52929531ebe30c9",
      "operation_type": "query",
      "partial_response": false,
      "payload_bytes": 86,
      "request_id": "[req_id]",
      "response_mode": "single",
      "status_code": 200,
      "subgraph_requests": 1,
      "supergraph_identifier": 0,
      "target": "router::request",
      "timestamp": "[timestamp]"
    }
    "#);

    // bad query
    let stdout_log = router
        .send_graphql_request("{", None, None)
        .capture_stdout_json()
        .await;
    let req_summary = log_line_by_target(&stdout_log, targets::SUMMARY);
    assert_json_snapshot!(req_summary, {
      ".timestamp" => "[timestamp]",
      ".request_id" => "[req_id]",
      ".duration_ms" => "[duration_ms]"
    }, @r#"
    {
      "duration_ms": "[duration_ms]",
      "error_code": "GRAPHQL_PARSE_FAILED",
      "error_count": 1,
      "involved_subgraphs": "",
      "level": "INFO",
      "partial_response": false,
      "payload_bytes": 167,
      "request_id": "[req_id]",
      "response_mode": "single",
      "status_code": 400,
      "subgraph_requests": 0,
      "supergraph_identifier": 0,
      "target": "router::request",
      "timestamp": "[timestamp]"
    }
    "#);

    // bad http req
    let stdout_log = router
        .send_post_request("/graphql", json!({}), None)
        .capture_stdout_json()
        .await;
    let req_summary = log_line_by_target(&stdout_log, targets::SUMMARY);
    assert_json_snapshot!(req_summary, {
      ".timestamp" => "[timestamp]",
      ".request_id" => "[req_id]",
      ".duration_ms" => "[duration_ms]"
    }, @r#"
    {
      "duration_ms": "[duration_ms]",
      "error_code": "MISSING_QUERY_PARAM",
      "error_count": 1,
      "involved_subgraphs": "",
      "level": "INFO",
      "partial_response": false,
      "payload_bytes": 101,
      "request_id": "[req_id]",
      "response_mode": "single",
      "status_code": 400,
      "subgraph_requests": 0,
      "supergraph_identifier": 0,
      "target": "router::request",
      "timestamp": "[timestamp]"
    }
    "#);
}

#[ntex::test]
async fn test_logging_of_subscriptions() {
    let subgraphs = TestSubgraphs::builder()
        .with_http_streaming_subscriptions_protocol(
            subgraphs::HTTPStreamingSubscriptionProtocol::SseOnly,
        )
        .build()
        .start()
        .await;

    let router = TestRouter::builder()
        .with_subgraphs(&subgraphs)
        .inline_config(
            r#"
          supergraph:
              source: file
              path: supergraph.graphql
          subscriptions:
              enabled: true
          log:
              level: debug
              format: json
          "#,
        )
        .build()
        .start()
        .await;

    let (stdout_log, res) = router
        .send_graphql_request(
            r#"
          subscription {
              reviewAdded(intervalInMs: 0) {
                  product {
                      upc
                  }
              }
          }
          "#,
            None,
            some_header_map! {
                http::header::ACCEPT => "text/event-stream"
            },
        )
        .capture_stdout_json_and_result()
        .await;

    assert_eq!(res.status(), 200, "Expected 200 OK");

    let lines = stdout_log
        .lines_json
        .iter()
        .filter(|v| v.get("target").is_some_and(|v| v == targets::SUMMARY))
        .collect::<Vec<_>>();

    assert_eq!(lines.len(), 1);

    assert_json_snapshot!(lines.get(0).unwrap(), {
      ".timestamp" => "[timestamp]",
      ".request_id" => "[req_id]",
      ".duration_ms" => "[duration_ms]"
    }, @r#"
    {
      "duration_ms": "[duration_ms]",
      "error_count": 0,
      "involved_subgraphs": "",
      "level": "INFO",
      "operation_hash": "a1424d79e11713f359a9772c62e72b7a",
      "operation_type": "subscription",
      "partial_response": false,
      "payload_bytes": -1,
      "request_id": "[req_id]",
      "response_mode": "stream",
      "status_code": 200,
      "subgraph_requests": 0,
      "supergraph_identifier": 0,
      "target": "router::request",
      "timestamp": "[timestamp]"
    }
    "#);
}

#[ntex::test]
async fn json_log_default_attrs() {
    let (_subgraphs, router) = setup_router(router_with_stdout_logging("json", "debug")).await;

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
      ".request_id" => "[req_id]"
    }, @r#"
    {
      "accept": "application/graphql-response+json",
      "content_type": "application/json",
      "level": "DEBUG",
      "message": "http request started",
      "method": "POST",
      "path": "/graphql",
      "query_string": "",
      "request_id": "[req_id]",
      "target": "router::http_server",
      "timestamp": "[timestamp]",
      "user_agent": ""
    }
    "#);

    assert_json_snapshot!(http_end, {
      ".request_id" => "[req_id]",
      ".timestamp" => "[timestamp]",
    }, @r#"
    {
      "level": "DEBUG",
      "message": "http request completed",
      "payload_bytes": 86,
      "request_id": "[req_id]",
      "status_code": 200,
      "target": "router::http_server",
      "timestamp": "[timestamp]"
    }
    "#);

    assert_json_snapshot!(gql_start, {
      ".timestamp" => "[timestamp]",
      ".request_id" => "[req_id]",
    }, @r#"
    {
      "body_size": 45,
      "level": "DEBUG",
      "message": "graphql request started",
      "operation": "{ users { id } }",
      "request_id": "[req_id]",
      "target": "router::graphql_execution",
      "timestamp": "[timestamp]"
    }
    "#);

    assert_json_snapshot!(gql_end, {
      ".timestamp" => "[timestamp]",
      ".request_id" => "[req_id]",
    }, @r#"
    {
      "error_count": 0,
      "level": "DEBUG",
      "message": "graphql request completed",
      "request_id": "[req_id]",
      "target": "router::graphql_execution",
      "timestamp": "[timestamp]"
    }
    "#);

    assert_json_snapshot!(subgraph_start, {
      ".timestamp" => "[timestamp]",
      ".request_id" => "[req_id]",
    }, @r#"
    {
      "dedupe": true,
      "executor": "http",
      "level": "DEBUG",
      "message": "executing subgraph request",
      "operation": "{users{id}}",
      "request_id": "[req_id]",
      "subgraph": "accounts",
      "target": "router::executor",
      "timestamp": "[timestamp]"
    }
    "#);

    assert_json_snapshot!(subgraph_end, {
      ".timestamp" => "[timestamp]",
      ".request_id" => "[req_id]",
    }, @r#"
    {
      "error_count": 0,
      "executor": "http",
      "http_status": 200,
      "level": "DEBUG",
      "message": "subgraph execution completed",
      "partial_response": false,
      "request_id": "[req_id]",
      "subgraph": "accounts",
      "target": "router::executor",
      "timestamp": "[timestamp]"
    }
    "#);
}

#[ntex::test]
async fn should_allow_to_customize_req_id_header_name() {
    let (_subgraphs, router) = setup_router(router_with_telemetry(
        "\
log:
  format: json
  correlation:
    id_header: x-ray-id
",
    ))
    .await;

    let stdout_log = router
        .send_graphql_request(TEST_QUERY, None, Some(single_header("x-request-id", "123")))
        .capture_stdout_json()
        .await;
    assert_ne!(
        req_id_of(log_line(&stdout_log, LOG_HTTP_REQUEST_START)),
        Some("123".to_string())
    );

    let stdout_log = router
        .send_graphql_request(TEST_QUERY, None, Some(single_header("x-ray-id", "123")))
        .capture_stdout_json()
        .await;
    assert_eq!(
        req_id_of(log_line(&stdout_log, LOG_HTTP_REQUEST_START)),
        Some("123".to_string())
    );
}

#[ntex::test]
async fn should_allow_to_disable_trace_id() {
    let (_subgraphs, router) = setup_router(router_with_telemetry(
        "\
log:
  format: json
  correlation:
    trace_propagation: false
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
    assert!(find_attr(http_start, "trace_id").is_none());
}
