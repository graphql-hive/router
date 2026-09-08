use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use libdd_trace_protobuf::pb::{ClientStatsPayload, Trilean};
use libdd_trace_utils::msgpack_decoder::v04;

use crate::testkit::{EnvVarsGuard, TestRouter, TestSubgraphs};

#[derive(Clone)]
struct AgentRequest {
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

struct MockDatadogAgent {
    address: String,
    requests: Arc<Mutex<Vec<AgentRequest>>>,
    _handle: std::thread::JoinHandle<()>,
}

impl MockDatadogAgent {
    fn start() -> Self {
        let server =
            tiny_http::Server::http("127.0.0.1:0").expect("failed to start mock datadog agent");
        let address = format!("http://{}", server.server_addr());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&requests);

        let handle = std::thread::spawn(move || {
            for mut request in server.incoming_requests() {
                let path = request.url().to_string();
                let headers = request
                    .headers()
                    .iter()
                    .map(|header| (header.field.to_string(), header.value.to_string()))
                    .collect();
                let mut body = Vec::new();
                request
                    .as_reader()
                    .read_to_end(&mut body)
                    .expect("failed to read datadog request");
                captured.lock().unwrap().push(AgentRequest {
                    path: path.clone(),
                    headers,
                    body,
                });

                // the info response enables native client-side stats, including
                // dropped traces
                let response = if path == "/info" {
                    r#"{"version":"7.0.0","endpoints":["/v0.4/traces","/v0.6/stats"],"client_drop_p0s":true}"#
                } else if path == "/v0.4/traces" {
                    r#"{"rate_by_service":{}}"#
                } else {
                    "{}"
                };
                let _ = request.respond(tiny_http::Response::from_string(response));
            }
        });

        Self {
            address,
            requests,
            _handle: handle,
        }
    }

    async fn wait_for_path(&self, path: &str) -> Vec<AgentRequest> {
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                let requests = self
                    .requests
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|request| request.path == path)
                    .cloned()
                    .collect::<Vec<_>>();
                if !requests.is_empty() {
                    return requests;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for datadog request to {path}"))
    }

    fn requests_for(&self, path: &str) -> Vec<AgentRequest> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|request| request.path == path)
            .cloned()
            .collect()
    }
}

fn supergraph_path() -> String {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("supergraph.graphql")
        .to_string_lossy()
        .into_owned()
}

#[ntex::test]
async fn test_datadog_sampled_trace_has_router_context() {
    let agent = MockDatadogAgent::start();
    // cover native agent discovery while keeping unrelated remote config and telemetry traffic out of the mock
    let _env = EnvVarsGuard::new()
        .set("DD_TRACE_AGENT_URL", &agent.address)
        .set("DD_REMOTE_CONFIGURATION_ENABLED", "false")
        .set("DD_INSTRUMENTATION_TELEMETRY_ENABLED", "false")
        .apply()
        .await;
    let subgraphs = TestSubgraphs::builder().build().start().await;
    let router = TestRouter::builder()
        .inline_config(format!(
            r#"
          supergraph:
            source: file
            path: {}
          telemetry:
            resource:
              attributes:
                custom.router: datadog-e2e
            tracing:
              collect:
                sampling: 1.0
              exporters:
                - kind: datadog
        "#,
            supergraph_path(),
        ))
        .with_subgraphs(&subgraphs)
        .build()
        .start()
        .await;

    agent.wait_for_path("/info").await;
    let response = router
        .send_graphql_request("query DatadogUsers { users { id } }", None, None)
        .await;
    assert!(response.status().is_success());
    drop(router);

    let requests = agent.wait_for_path("/v0.4/traces").await;
    let request = requests
        .iter()
        .find(|request| {
            v04::from_slice(&request.body).is_ok_and(|(traces, _)| {
                traces
                    .iter()
                    .flatten()
                    .any(|span| span.meta.get("hive.kind").copied() == Some("graphql.operation"))
            })
        })
        .expect("datadog did not receive the graphql trace");
    assert!(request.headers.iter().any(|(name, value)| {
        name.eq_ignore_ascii_case("content-type") && value == "application/msgpack"
    }));
    assert!(request
        .headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("datadog-client-computed-stats")));

    let (traces, _) = v04::from_slice(&request.body).expect("invalid datadog trace payload");
    let trace = traces
        .iter()
        .find(|trace| {
            trace
                .iter()
                .any(|span| span.meta.get("hive.kind").copied() == Some("graphql.operation"))
        })
        .unwrap();
    let root = trace.iter().find(|span| span.parent_id == 0).unwrap();
    assert_eq!(root.name, "http.server.request");
    assert_eq!(root.resource, "POST /graphql");
    assert_eq!(root.service, "hive-router");
    assert_eq!(root.meta.get("span.kind").copied(), Some("server"));
    assert_eq!(root.meta.get("http.route").copied(), Some("/graphql"));
    assert_eq!(root.meta.get("custom.router").copied(), Some("datadog-e2e"));
    assert_eq!(
        root.meta.get("telemetry.sdk.name").copied(),
        Some("datadog")
    );

    let operation = trace
        .iter()
        .find(|span| span.meta.get("hive.kind").copied() == Some("graphql.operation"))
        .unwrap();
    assert_eq!(operation.name, "graphql.server.request");
    assert_eq!(operation.resource, "query DatadogUsers");
    assert_eq!(
        operation.meta.get("graphql.operation.name").copied(),
        Some("DatadogUsers")
    );
    assert_eq!(
        operation.meta.get("graphql.operation.type").copied(),
        Some("query")
    );
    assert!(operation.meta.get("graphql.document.hash").is_some());
    assert!(
        operation.meta.get("graphql.document").is_none(),
        "graphql documents must stay out of datadog unless explicitly enabled"
    );
}

#[ntex::test]
async fn test_datadog_zero_sampling_reports_every_root_in_stats_only() {
    // remote config polling and tracer telemetry use unrelated agent endpoints
    let _env = EnvVarsGuard::new()
        .set("DD_REMOTE_CONFIGURATION_ENABLED", "false")
        .set("DD_INSTRUMENTATION_TELEMETRY_ENABLED", "false")
        .apply()
        .await;
    let agent = MockDatadogAgent::start();
    let subgraphs = TestSubgraphs::builder().build().start().await;
    let router = TestRouter::builder()
        .inline_config(format!(
            r#"
          supergraph:
            source: file
            path: {}
          telemetry:
            tracing:
              collect:
                sampling: 0.0
              exporters:
                - kind: datadog
                  endpoint: {}
        "#,
            supergraph_path(),
            agent.address,
        ))
        .with_subgraphs(&subgraphs)
        .build()
        .start()
        .await;

    agent.wait_for_path("/info").await;
    for _ in 0..3 {
        let response = router
            .send_graphql_request("query DatadogUsers { users { id } }", None, None)
            .await;
        assert!(response.status().is_success());
    }
    drop(router);

    let stats_requests = agent.wait_for_path("/v0.6/stats").await;
    let root_stats = stats_requests
        .iter()
        .map(|request| {
            rmp_serde::from_slice::<ClientStatsPayload>(&request.body)
                .expect("invalid datadog stats payload")
        })
        .flat_map(|payload| payload.stats)
        .flat_map(|bucket| bucket.stats)
        .filter(|stats| {
            stats.name == "http.server.request"
                && stats.resource == "POST /graphql"
                && stats.span_kind == "server"
                && stats.is_trace_root == Trilean::True as i32
        })
        .collect::<Vec<_>>();

    assert_eq!(root_stats.iter().map(|stats| stats.hits).sum::<u64>(), 3);
    assert_eq!(root_stats.iter().map(|stats| stats.errors).sum::<u64>(), 0);
    assert!(root_stats.iter().all(|stats| {
        stats.service == "hive-router" && stats.http_status_code == 200 && stats.duration > 0
    }));

    // record-only requests feed stats but must never appear as retained trace payloads.
    assert!(agent.requests_for("/v0.4/traces").iter().all(|request| {
        v04::from_slice(&request.body)
            .map(|(traces, _)| traces.is_empty())
            .unwrap_or(false)
    }));
}

#[ntex::test]
async fn test_datadog_partial_sampling_retains_some_traces_and_stats_cover_every_root() {
    const REQUEST_COUNT: u64 = 100;

    // remote config polling and tracer telemetry use unrelated agent endpoints
    let _env = EnvVarsGuard::new()
        .set("DD_REMOTE_CONFIGURATION_ENABLED", "false")
        .set("DD_INSTRUMENTATION_TELEMETRY_ENABLED", "false")
        .apply()
        .await;
    let agent = MockDatadogAgent::start();
    let subgraphs = TestSubgraphs::builder().build().start().await;
    let router = TestRouter::builder()
        .inline_config(format!(
            r#"
          supergraph:
            source: file
            path: {}
          telemetry:
            tracing:
              collect:
                sampling: 0.5
              exporters:
                - kind: datadog
                  endpoint: {}
        "#,
            supergraph_path(),
            agent.address,
        ))
        .with_subgraphs(&subgraphs)
        .build()
        .start()
        .await;

    agent.wait_for_path("/info").await;
    for _ in 0..REQUEST_COUNT {
        let response = router
            .send_graphql_request("query DatadogUsers { users { id } }", None, None)
            .await;
        assert!(response.status().is_success());
    }
    drop(router);

    let stats_requests = agent.wait_for_path("/v0.6/stats").await;
    let root_hits = stats_requests
        .iter()
        .map(|request| {
            rmp_serde::from_slice::<ClientStatsPayload>(&request.body)
                .expect("invalid datadog stats payload")
        })
        .flat_map(|payload| payload.stats)
        .flat_map(|bucket| bucket.stats)
        .filter(|stats| {
            stats.name == "http.server.request"
                && stats.resource == "POST /graphql"
                && stats.span_kind == "server"
                && stats.is_trace_root == Trilean::True as i32
        })
        .map(|stats| stats.hits)
        .sum::<u64>();
    assert_eq!(root_hits, REQUEST_COUNT);

    // a partial rate must retain detail for only a subset while stats still count every request.
    let retained = agent
        .requests_for("/v0.4/traces")
        .iter()
        .map(|request| {
            v04::from_slice(&request.body)
                .expect("invalid datadog trace payload")
                .0
        })
        .flatten()
        .filter(|trace| {
            trace
                .iter()
                .any(|span| span.meta.get("hive.kind").copied() == Some("graphql.operation"))
        })
        .count() as u64;
    assert!(retained > 0, "partial sampling retained no traces");
    assert!(
        retained < REQUEST_COUNT,
        "partial sampling retained every trace"
    );
}
