#[cfg(test)]
mod graceful_shutdown_tests {
    use std::{
        path::Path,
        time::{Duration, Instant},
    };

    use sonic_rs::{json, JsonValueTrait};
    use tokio::net::TcpStream;

    use crate::testkit::{
        get_available_port, supergraph_temp_file_with_subgraphs, ClientResponseExt, TestRouter,
        TestSubgraphs,
    };

    /// Graceful shutdown (`ntex`'s `Server::stop(true)`, triggered on `SIGTERM` in
    /// production) must let in-flight requests finish rather than dropping them.
    #[ntex::test]
    async fn should_complete_an_in_flight_request_during_graceful_shutdown() {
        let subgraphs = TestSubgraphs::builder()
            .with_delay(Duration::from_secs(2))
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
                "#,
            )
            .build()
            .start()
            .await;

        let request = router.send_graphql_request("{ topProducts { name price } }", None, None);
        let shutdown = router.serv().stop();

        // Kick off the slow request first so shutdown starts while it is in flight.
        let (res, ()) = tokio::join!(request, async {
            tokio::time::sleep(Duration::from_millis(200)).await;
            shutdown.await;
        });

        assert!(
            res.status().is_success(),
            "expected the in-flight request to complete despite a graceful shutdown \
             starting mid-flight, got status {}",
            res.status()
        );
        let json_body = res.json_body().await;
        assert!(
            json_body["data"]["topProducts"].is_array(),
            "expected a full GraphQL response, got: {json_body:?}"
        );
    }

    /// Once graceful shutdown has completed, the listener must be gone: new connection
    /// attempts should fail rather than hang or be silently accepted.
    #[ntex::test]
    async fn should_stop_accepting_new_connections_after_graceful_shutdown_completes() {
        let router = TestRouter::builder()
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                "#,
            )
            .build()
            .start()
            .await;

        let addr = router.serv().addr();
        router.serv().stop().await;

        let connect_result =
            tokio::time::timeout(Duration::from_secs(3), TcpStream::connect(addr)).await;
        assert!(
            matches!(connect_result, Ok(Err(_))) || connect_result.is_err(),
            "expected the router to refuse new connections after graceful shutdown completed, \
             got: {connect_result:?}"
        );
    }

    /// The `test::TestServer` used by the tests above always drains with ntex's internal
    /// default timeout and ignores `http.shutdown_timeout` entirely, so it can't prove the
    /// configured value is what actually governs the drain. This test binds the router with
    /// the real `web::HttpServer` (via `with_real_http_server`) and configures a
    /// `shutdown_timeout` far shorter than the in-flight request: if the configured value is
    /// honored, the request must be force-dropped around that short timeout rather than
    /// after the request's own 3s delay, and shutdown itself must complete around that same
    /// short timeout instead of ntex's 30s default.
    #[ntex::test]
    async fn should_honor_the_configured_shutdown_timeout() {
        let subgraphs = TestSubgraphs::builder()
            .with_delay(Duration::from_secs(3))
            .build()
            .start()
            .await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .with_real_http_server()
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                http:
                    shutdown_timeout: 500ms
                "#,
            )
            .build()
            .start_without_healthcheck()
            .await;

        let client = reqwest::Client::new();
        let url = router.real_serv().url("/graphql");
        // Spawned so it actually starts executing concurrently: an un-awaited reqwest
        // future does nothing on its own.
        let request_handle = tokio::spawn(async move {
            client
                .post(url)
                .header("content-type", "application/json")
                .header("accept", "application/graphql-response+json")
                .json(&json!({ "query": "{ topProducts { name price } }" }))
                .send()
                .await
        });

        // Give the request a moment to actually reach the slow subgraph call before
        // shutdown starts draining.
        tokio::time::sleep(Duration::from_millis(150)).await;
        let shutdown_started = Instant::now();
        router.real_serv().stop().await;
        let shutdown_elapsed = shutdown_started.elapsed();

        assert!(
            shutdown_elapsed < Duration::from_secs(2),
            "expected graceful shutdown to force-drop the connection around the configured \
             500ms http.shutdown_timeout, not wait out the request's 3s subgraph delay or \
             ntex's 30s default, but shutdown took {shutdown_elapsed:?}"
        );

        // Bounded independently of `stop()`: if the connection was left dangling instead of
        // force-dropped, this fails the test instead of hanging it.
        if let Ok(Ok(Ok(res))) = tokio::time::timeout(Duration::from_secs(2), request_handle).await
        {
            assert!(
                !res.status().is_success(),
                "expected the in-flight request to be force-dropped once the configured \
                 500ms shutdown_timeout elapsed, well before the subgraph's 3s delayed \
                 response, but it completed successfully"
            );
        }
    }

    /// Regression test for the class of bug fixed in #1445: `ntex-server` made graceful
    /// signal handling opt-in (default off), but `ntex::web::HttpServer` has no way to opt
    /// in, so a real `SIGTERM` silently force-drops in-flight requests and ignores
    /// `http.shutdown_timeout`. The tests above all call `Server::stop(true)` directly,
    /// which always requests a graceful stop regardless of that flag and so cannot catch
    /// this — they exercise the drain mechanism, not the SIGTERM-to-graceful-flag wiring
    /// that actually broke. This test spawns the real compiled router binary and sends it
    /// an actual `SIGTERM`, exercising the same signal-handling path production relies on.
    ///
    /// Ignored by default: it requires the `hive_router` binary to be pre-built (see the
    /// panic message below), which local `cargo test_e2e` runs don't do. CI's `e2e` job
    /// builds it and runs with `--include-ignored` so this still runs there.
    #[ntex::test]
    #[ignore = "painful to run locally, but that's the only real way to test a real shutdown through SIGTERM"]
    async fn should_gracefully_drain_in_flight_requests_on_real_sigterm() {
        // This deliberately does NOT build the binary itself: invoking `cargo build` from
        // inside a test spawned by `cargo nextest run` fights nextest's own build/process
        // management (nested cargo invocations, and nextest's default 30s slow-test kill
        // routinely fires before a cold build finishes). CI builds this binary as a
        // separate step (see `.github/workflows/ci.yaml`); locally, build it once with:
        //   cargo build -p hive-router --bin hive_router --features testing
        let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("e2e crate must be a workspace member");
        let binary_path = workspace_root.join("target/debug/hive_router");
        assert!(
            binary_path.exists(),
            "hive_router binary not found at {binary_path:?} — build it first with: \
             cargo build -p hive-router --bin hive_router --features testing"
        );

        let subgraphs = TestSubgraphs::builder()
            .with_delay(Duration::from_secs(2))
            .build()
            .start()
            .await;

        let supergraph_path = concat!(env!("CARGO_MANIFEST_DIR"), "/supergraph.graphql");
        let supergraph_temp_path =
            supergraph_temp_file_with_subgraphs(supergraph_path, &subgraphs.url());

        let port = get_available_port();

        let mut child = tokio::process::Command::new(&binary_path)
            .env("SUPERGRAPH_FILE_PATH", &supergraph_temp_path)
            .env("PORT", port.to_string())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .expect("failed to spawn hive_router subprocess");
        let pid = child.id().expect("spawned child must have a pid");

        let client = reqwest::Client::new();
        let health_url = format!("http://127.0.0.1:{port}/health");
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                if let Ok(res) = client.get(&health_url).send().await {
                    if res.status().is_success() {
                        break;
                    }
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        })
        .await
        .expect("hive_router subprocess did not become healthy in time");

        let graphql_url = format!("http://127.0.0.1:{port}/graphql");
        // Spawned so it actually starts executing concurrently: an un-awaited reqwest
        // future does nothing on its own.
        let request_handle = tokio::spawn(async move {
            reqwest::Client::new()
                .post(graphql_url)
                .header("content-type", "application/json")
                .header("accept", "application/graphql-response+json")
                .json(&json!({ "query": "{ topProducts { name price } }" }))
                .send()
                .await
        });

        // Give the request a moment to actually reach the slow subgraph call before SIGTERM.
        tokio::time::sleep(Duration::from_millis(300)).await;

        let sigterm_sent = Instant::now();
        let kill_status = std::process::Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .status()
            .expect("failed to send SIGTERM to the hive_router subprocess");
        assert!(kill_status.success(), "kill -TERM failed");

        let exit_status = tokio::time::timeout(Duration::from_secs(15), child.wait())
            .await
            .expect("hive_router subprocess did not exit after SIGTERM")
            .expect("failed to wait on hive_router subprocess");
        let exit_elapsed = sigterm_sent.elapsed();

        // The clearest signal of the regression from #1445's own measurements: on the
        // broken ntex-server, the process exits well under a second after SIGTERM
        // (a forced, non-graceful stop). A graceful stop stays alive until the 2s
        // in-flight request finishes draining.
        assert!(
            exit_elapsed >= Duration::from_millis(1500),
            "expected the process to stay alive until the 2s in-flight request finished \
             draining, but it exited only {exit_elapsed:?} after SIGTERM \
             (a near-instant exit means SIGTERM force-dropped the connection instead of \
             gracefully draining it)"
        );
        assert!(
            exit_status.success(),
            "expected hive_router to exit cleanly after a graceful SIGTERM shutdown, got {exit_status:?}"
        );

        let res = tokio::time::timeout(Duration::from_secs(5), request_handle)
            .await
            .expect("timed out waiting for the in-flight request")
            .expect("request task panicked")
            .expect("request errored instead of completing");

        assert!(
            res.status().is_success(),
            "expected the in-flight request to complete successfully despite SIGTERM \
             arriving mid-flight, got status {}",
            res.status()
        );
        let body: sonic_rs::Value =
            sonic_rs::from_str(&res.text().await.expect("failed to read response body"))
                .expect("failed to parse response body as JSON");
        assert!(
            body["data"]["topProducts"].is_array(),
            "expected a full GraphQL response, got: {body:?}"
        );
    }
}
