#[cfg(test)]
mod mutation_execution_e2e_tests {
    use std::{
        net::SocketAddr,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        },
        time::{Duration, Instant},
    };

    use axum::{extract::State, http::StatusCode, routing::post, Json, Router};
    use bytes::Bytes;
    use hive_router::pipeline::execution::EXPOSE_QUERY_PLAN_HEADER;
    use sonic_rs::{JsonContainerTrait, JsonValueTrait};

    use crate::testkit::{some_header_map, ClientResponseExt, TestRouter};

    const OPERATION: &str = r#"
        mutation MutationBarrierRegression {
          first: selectItem(id: "1") {
            id
            value
          }
          second: checkpoint
          third: updateItem(id: "1", value: "after")
        }
    "#;

    struct BarrierState {
        item_value: tokio::sync::RwLock<String>,
        events: tokio::sync::Mutex<Vec<String>>,
        e1_release: tokio::sync::Notify,
        first_count: AtomicUsize,
        second_count: AtomicUsize,
        third_count: AtomicUsize,
        items_count: AtomicUsize,
    }

    impl BarrierState {
        fn new() -> Self {
            Self {
                item_value: tokio::sync::RwLock::new("before".to_string()),
                events: tokio::sync::Mutex::new(Vec::new()),
                e1_release: tokio::sync::Notify::new(),
                first_count: AtomicUsize::new(0),
                second_count: AtomicUsize::new(0),
                third_count: AtomicUsize::new(0),
                items_count: AtomicUsize::new(0),
            }
        }

        async fn push_event(&self, event: String) {
            let mut events = self.events.lock().await;
            events.push(event);
        }

        async fn snapshot_events(&self) -> Vec<String> {
            self.events.lock().await.clone()
        }
    }

    async fn first_handler(
        State(state): State<Arc<BarrierState>>,
        _body: Bytes,
    ) -> impl axum::response::IntoResponse {
        state.first_count.fetch_add(1, Ordering::SeqCst);
        state.push_event("M1_STARTED".to_string()).await;
        let body = serde_json::json!({
            "data": {
                "first": {
                    "__typename": "Item",
                    "id": "1"
                }
            }
        });
        (StatusCode::OK, Json(body))
    }

    async fn second_handler(
        State(state): State<Arc<BarrierState>>,
        _body: Bytes,
    ) -> impl axum::response::IntoResponse {
        state.second_count.fetch_add(1, Ordering::SeqCst);
        state.push_event("M2_STARTED".to_string()).await;
        // Returns immediately; the test's bounded wait proves the router has time to
        // consume this response while E1 is still gated.
        let body = serde_json::json!({
            "data": {
                "second": true
            }
        });
        state.push_event("M2_RETURNED".to_string()).await;
        (StatusCode::OK, Json(body))
    }

    async fn third_handler(
        State(state): State<Arc<BarrierState>>,
        _body: Bytes,
    ) -> impl axum::response::IntoResponse {
        state.third_count.fetch_add(1, Ordering::SeqCst);
        state.push_event("M3_STARTED".to_string()).await;
        {
            let mut value = state.item_value.write().await;
            *value = "after".to_string();
        }
        let body = serde_json::json!({
            "data": {
                "third": true
            }
        });
        (StatusCode::OK, Json(body))
    }

    async fn items_handler(
        State(state): State<Arc<BarrierState>>,
        _body: Bytes,
    ) -> impl axum::response::IntoResponse {
        state.items_count.fetch_add(1, Ordering::SeqCst);
        state.push_event("E1_STARTED".to_string()).await;
        // Block until the test releases the gate. Timeout so a test failure cannot
        // deadlock the router request forever; on timeout we still proceed so the
        // request can finish and the test can report the real assertion failure.
        let _ = tokio::time::timeout(Duration::from_secs(10), state.e1_release.notified()).await;
        // Critical: read shared state AFTER the gate opens. Reading before waiting
        // would conceal the response corruption this regression protects against.
        let value = state.item_value.read().await.clone();
        state.push_event(format!("E1_READ:{value}")).await;
        let body = serde_json::json!({
            "data": {
                "_entities": [
                    {
                        "__typename": "Item",
                        "value": value
                    }
                ]
            }
        });
        (StatusCode::OK, Json(body))
    }

    async fn start_barrier_subgraphs(
        state: Arc<BarrierState>,
    ) -> (SocketAddr, tokio::task::JoinHandle<()>) {
        let app = Router::new()
            .route("/first", post(first_handler))
            .route("/second", post(second_handler))
            .route("/third", post(third_handler))
            .route("/items", post(items_handler))
            .with_state(state);

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("failed to bind barrier subgraphs listener");
        let addr = listener.local_addr().expect("failed to get local addr");
        let handle = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("barrier subgraphs server failed");
        });
        // The listener is already bound, but give the serve task a moment to start
        // accepting before the router sends its first fetch.
        tokio::time::sleep(Duration::from_millis(50)).await;
        (addr, handle)
    }

    fn index_of(events: &[String], needle: &str) -> Option<usize> {
        events.iter().position(|e| e == needle)
    }

    fn index_of_prefix(events: &[String], prefix: &str) -> Option<usize> {
        events.iter().position(|e| e.starts_with(prefix))
    }

    async fn wait_for_event(
        state: &BarrierState,
        predicate: impl Fn(&[String]) -> bool,
        timeout: Duration,
    ) -> bool {
        let start = Instant::now();
        while start.elapsed() < timeout {
            {
                let events = state.events.lock().await;
                if predicate(&events) {
                    return true;
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        // Final check after timeout.
        let events = state.events.lock().await;
        predicate(&events)
    }

    async fn run_barrier_case(dependency_aware: bool) {
        let state = Arc::new(BarrierState::new());
        let (addr, server_handle) = start_barrier_subgraphs(state.clone()).await;

        let router = TestRouter::builder()
            .inline_config(format!(
                r#"
                  supergraph:
                    source: file
                    path: src/mutation_execution_supergraph.graphql
                  query_planner:
                    allow_expose: true
                  execution:
                    experimental_dependency_aware_execution: {dependency_aware}
                  override_subgraph_urls:
                    subgraphs:
                      first:
                        url: "http://{addr}/first"
                      second:
                        url: "http://{addr}/second"
                      third:
                        url: "http://{addr}/third"
                      items:
                        url: "http://{addr}/items"
                  "#
            ))
            .build()
            .start()
            .await;

        // Verify the fixture demonstrably exercises:
        //   M1 ──→ E1
        //    └──→ M2 ──→ M3
        // i.e. Sequence(M1, Parallel(E1, M2), M3). Without this, planner merging or
        // extra dependencies could make the test pass without covering the bug.
        let plan_res = router
            .send_graphql_request(
                OPERATION,
                None,
                some_header_map! {
                    http::header::HeaderName::from_static(EXPOSE_QUERY_PLAN_HEADER.as_str()) => "dry-run"
                },
            )
            .await;
        assert!(
            plan_res.status().is_success(),
            "dry-run plan request failed"
        );
        let plan_body = plan_res.json_body().await;
        let plan_node = &plan_body["extensions"]["queryPlan"]["node"];
        assert!(
            plan_node.is_object(),
            "expected queryPlan in dry-run response, got: {plan_body:?}"
        );

        // Top level must be Sequence(M1, Parallel(...), M3).
        assert_eq!(
            plan_node["kind"].as_str(),
            Some("Sequence"),
            "expected top-level Sequence, got: {plan_node:?}"
        );
        let seq_nodes = plan_node["nodes"].as_array().expect("Sequence nodes");
        assert_eq!(
            seq_nodes.len(),
            3,
            "expected Sequence(M1, Parallel, M3), got: {plan_node:?}"
        );
        assert_eq!(
            seq_nodes[0]["serviceName"].as_str(),
            Some("first"),
            "expected M1 (first) first, got: {plan_node:?}"
        );
        assert_eq!(
            seq_nodes[0]["operationKind"].as_str(),
            Some("mutation"),
            "expected M1 to be a mutation fetch, got: {plan_node:?}"
        );
        assert_eq!(
            seq_nodes[1]["kind"].as_str(),
            Some("Parallel"),
            "expected Parallel(E1, M2) second, got: {plan_node:?}"
        );
        let parallel_nodes = seq_nodes[1]["nodes"].as_array().expect("Parallel nodes");
        assert_eq!(
            parallel_nodes.len(),
            2,
            "expected Parallel with E1 and M2, got: {plan_node:?}"
        );
        let mut parallel_services: Vec<&str> = parallel_nodes
            .iter()
            .map(|n| {
                // E1 is a Flatten wrapping a Fetch; M2 is a plain Fetch.
                if n["kind"].as_str() == Some("Flatten") {
                    n["node"]["serviceName"].as_str().unwrap_or("<missing>")
                } else {
                    n["serviceName"].as_str().unwrap_or("<missing>")
                }
            })
            .collect();
        parallel_services.sort_unstable();
        assert_eq!(
            parallel_services,
            vec!["items", "second"],
            "expected Parallel(items=E1, second=M2), got: {plan_node:?}"
        );
        assert_eq!(
            seq_nodes[2]["serviceName"].as_str(),
            Some("third"),
            "expected M3 (third) last, got: {plan_node:?}"
        );

        // Dry-run must not have touched the subgraphs.
        assert_eq!(state.first_count.load(Ordering::SeqCst), 0);
        assert_eq!(state.second_count.load(Ordering::SeqCst), 0);
        assert_eq!(state.third_count.load(Ordering::SeqCst), 0);
        assert_eq!(state.items_count.load(Ordering::SeqCst), 0);

        // Spawn the gatekeeper: waits for E1 to block, holds it while giving the
        // executor time to (incorrectly) start M3, then releases even if a
        // violation was observed so the request can finish and we can check for
        // response corruption.
        let gate_state = state.clone();
        let gatekeeper = tokio::spawn(async move {
            let e1_started = wait_for_event(
                &gate_state,
                |events| events.iter().any(|e| e == "E1_STARTED"),
                Duration::from_secs(10),
            )
            .await;
            assert!(e1_started, "E1 never started; plan may not match fixture");

            // Bounded wait for the forbidden M3_STARTED while E1 is blocked.
            // Correct execution deadlocks if we release only when M3 starts, so
            // we always release after this window.
            let window_end = Instant::now() + Duration::from_millis(800);
            let mut violation_during_window = false;
            while Instant::now() < window_end {
                {
                    let events = gate_state.events.lock().await;
                    if events.iter().any(|e| e == "M3_STARTED") {
                        violation_during_window = true;
                        break;
                    }
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }

            gate_state.e1_release.notify_waiters();
            violation_during_window
        });

        // Run the actual mutation while the gatekeeper holds E1.
        let overall = tokio::time::timeout(
            Duration::from_secs(20),
            router.send_graphql_request(OPERATION, None, None),
        )
        .await;
        // Ensure failure paths release the gate and clean up the request.
        state.e1_release.notify_waiters();

        let violation_during_window = gatekeeper.await.expect("gatekeeper panicked");

        let res = overall.expect("router request timed out (E1 gate may not have been released)");
        assert!(res.status().is_success(), "Expected 200 OK");

        let json_body = res.json_body().await;
        let events = state.snapshot_events().await;

        // 1. No M3_STARTED while E1 is blocked.
        assert!(
            !violation_during_window,
            "M3 started while E1 was blocked (dependency-aware broke the mutation wave barrier). events: {events:?}"
        );

        // 2. E1_READ precedes M3_STARTED in the final log.
        let e1_read_idx = index_of_prefix(&events, "E1_READ:");
        let m3_idx = index_of(&events, "M3_STARTED");
        assert!(
            e1_read_idx.is_some() && m3_idx.is_some(),
            "expected both E1_READ and M3_STARTED, got: {events:?}"
        );
        assert!(
            e1_read_idx.unwrap() < m3_idx.unwrap(),
            "E1_READ must precede M3_STARTED to preserve the wave barrier. events: {events:?}"
        );

        // M2 must not have waited for E1 (the wave allows Parallel(E1, M2)).
        // Its handler returns immediately, so it should complete before E1 is released.
        let m2_returned_idx = index_of(&events, "M2_RETURNED");
        assert!(
            m2_returned_idx.is_some(),
            "expected M2_RETURNED, got: {events:?}"
        );
        assert!(
            m2_returned_idx.unwrap() < e1_read_idx.unwrap(),
            "M2 should complete while E1 is blocked (Parallel). events: {events:?}"
        );

        // 3. Exact response: E1 must have read "before"; a barrier violation lets
        // M3 run first and corrupts first.value to "after".
        assert!(
            events.iter().any(|e| e == "E1_READ:before"),
            "E1 must read value before M3 updates it, got: {events:?}"
        );
        assert!(
            json_body["errors"].is_null(),
            "expected no GraphQL errors, got: {json_body:?}"
        );
        assert_eq!(
            json_body["data"]["first"]["id"].as_str(),
            Some("1"),
            "first.id mismatch. body: {json_body:?}, events: {events:?}"
        );
        assert_eq!(
            json_body["data"]["first"]["value"].as_str(),
            Some("before"),
            "first.value corrupted (M3 ran before E1 finished). body: {json_body:?}, events: {events:?}"
        );
        assert_eq!(
            json_body["data"]["second"].as_bool(),
            Some(true),
            "second mismatch. body: {json_body:?}"
        );
        assert_eq!(
            json_body["data"]["third"].as_bool(),
            Some(true),
            "third mismatch. body: {json_body:?}"
        );

        // 4. Final shared state proves M3 actually ran.
        let final_value = state.item_value.read().await.clone();
        assert_eq!(
            final_value, "after",
            "final item state should be 'after' (M3 ran). events: {events:?}"
        );

        // 5. Each of the four expected fetches executes once.
        assert_eq!(
            state.first_count.load(Ordering::SeqCst),
            1,
            "M1 should execute once. events: {events:?}"
        );
        assert_eq!(
            state.second_count.load(Ordering::SeqCst),
            1,
            "M2 should execute once. events: {events:?}"
        );
        assert_eq!(
            state.third_count.load(Ordering::SeqCst),
            1,
            "M3 should execute once. events: {events:?}"
        );
        assert_eq!(
            state.items_count.load(Ordering::SeqCst),
            1,
            "E1 should execute once. events: {events:?}"
        );

        server_handle.abort();
    }

    #[ntex::test]
    async fn dependency_aware_execution_preserves_mutation_wave_barriers_flag_off() {
        run_barrier_case(false).await;
    }

    #[ntex::test]
    async fn dependency_aware_execution_preserves_mutation_wave_barriers_flag_on() {
        run_barrier_case(true).await;
    }
}
