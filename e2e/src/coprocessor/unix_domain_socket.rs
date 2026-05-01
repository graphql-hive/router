use std::path::PathBuf;

use axum::{routing::post, Json, Router};
use serde_json::{json, Value};
use tempfile::Builder;
use tokio::fs::remove_file;
use tokio::net::UnixListener;
use tokio::sync::oneshot;

use crate::testkit::{TestRouter, TestSubgraphs};

#[ntex::test]
async fn works_over_unix_domain_socket() {
    run_unix_domain_socket_test("http1").await;
}

#[ntex::test]
async fn works_over_unix_domain_socket_h2c() {
    run_unix_domain_socket_test("h2c").await;
}

async fn run_unix_domain_socket_test(protocol: &str) {
    let subgraphs = TestSubgraphs::builder().build().start().await;

    let socket_dir = Builder::new()
        .prefix("hive-coprocessor-")
        .tempdir_in("/tmp")
        .expect("failed to create temporary socket directory");
    let socket_path_buf = socket_dir.path().join("coprocessor.sock");
    let request_path = "/coprocessor";

    let (shutdown_tx, shutdown_rx) = oneshot::channel();

    let server_handle = tokio::spawn(run_unix_coprocessor(
        socket_path_buf.clone(),
        request_path,
        shutdown_rx,
    ));

    let router = TestRouter::builder()
        .with_subgraphs(&subgraphs)
        .inline_config(format!(
            r#"
                supergraph:
                  source: file
                  path: supergraph.graphql
                coprocessor:
                  url: unix://{}?path={request_path}
                  protocol: {protocol}
                  stages:
                    graphql:
                      analysis:
                        include:
                          body: [query]
                    router:
                      response:
                        include:
                          status_code: true
                "#,
            socket_path_buf.display()
        ))
        .build()
        .start()
        .await;

    let ok_response = router
        .send_graphql_request("{ topProducts { name } }", None, None)
        .await;

    assert!(
        ok_response.status().is_success(),
        "expected successful response"
    );
    assert_eq!(
        ok_response
            .headers()
            .get("x-coprocessor-response")
            .and_then(|v| v.to_str().ok()),
        Some("injected-by-router-response")
    );

    let blocked_response = router
        .send_graphql_request("{ __schema { queryType { name } } }", None, None)
        .await;

    assert_eq!(blocked_response.status().as_u16(), 403);
    assert_eq!(
        blocked_response
            .headers()
            .get("x-coprocessor-reason")
            .and_then(|v| v.to_str().ok()),
        Some("blocked-by-graphql-analysis")
    );

    let _ = shutdown_tx.send(());
    let _ = server_handle.await;
}

async fn run_unix_coprocessor(
    socket_path: PathBuf,
    request_path: &'static str,
    shutdown_rx: oneshot::Receiver<()>,
) {
    if socket_path.exists() {
        let _ = remove_file(&socket_path);
    }

    let listener = UnixListener::bind(&socket_path).expect("failed to bind unix socket");

    let app = Router::new().route(request_path, post(coprocessor_handler));

    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = shutdown_rx.await;
        })
        .await
        .expect("failed to serve unix coprocessor");

    let _ = remove_file(&socket_path);
}

async fn coprocessor_handler(Json(payload): Json<Value>) -> Json<Value> {
    let stage = payload
        .get("stage")
        .and_then(Value::as_str)
        .unwrap_or_default();

    match stage {
        "graphql.analysis" => {
            let query = payload
                .pointer("/body/query")
                .and_then(Value::as_str)
                .unwrap_or_default();

            if query.contains("__schema") {
                return Json(json!({
                    "version": 1,
                    "control": { "break": 403 },
                    "headers": {
                        "content-type": "application/json",
                        "x-coprocessor-reason": "blocked-by-graphql-analysis"
                    },
                    "body": {
                        "errors": [
                            { "message": "Operation rejected by policy" }
                        ]
                    }
                }));
            }
        }
        "router.response" => {
            let status_code = payload
                .get("status_code")
                .and_then(Value::as_u64)
                .unwrap_or_default();

            if status_code == 200 {
                return Json(json!({
                    "version": 1,
                    "control": "continue",
                    "headers": {
                        "content-type": "application/json",
                        "x-coprocessor-response": "injected-by-router-response"
                    }
                }));
            }
        }
        _ => {}
    };

    Json(json!({ "version": 1, "control": "continue" }))
}
