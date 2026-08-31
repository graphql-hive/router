#[cfg(test)]
mod graceful_shutdown_tests {
    use std::time::{Duration, Instant};

    use sonic_rs::{json, JsonValueTrait};
    use tokio::net::TcpStream;

    use crate::testkit::{ClientResponseExt, TestRouter, TestSubgraphs};

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
}
