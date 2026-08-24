#[cfg(test)]
mod keep_alive_tests {
    use std::time::Duration;

    use sonic_rs::JsonValueTrait;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpStream,
        time::timeout,
    };

    use crate::testkit::{ClientResponseExt, TestRouter, TestSubgraphs};

    const KEEP_ALIVE_REQUEST: &[u8] =
        b"GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: keep-alive\r\n\r\n";

    /// Sends a request on `stream` and returns the response bytes, or `None` if the
    /// write failed or the connection was already closed by the server (EOF).
    async fn try_request(stream: &mut TcpStream) -> Option<Vec<u8>> {
        if stream.write_all(KEEP_ALIVE_REQUEST).await.is_err() {
            return None;
        }

        let mut buf = vec![0u8; 4096];
        match timeout(Duration::from_secs(3), stream.read(&mut buf)).await {
            Ok(Ok(0)) | Ok(Err(_)) | Err(_) => None,
            Ok(Ok(n)) => Some(buf[..n].to_vec()),
        }
    }

    /// Regression test for `http.keep_alive`
    ///
    /// Configures a short 1s keep-alive and expects
    /// the router to close an idle connection shortly after that, well before
    /// `traffic_shaping.router.request_timeout` (60s by default) would kick in.
    #[ntex::test]
    async fn should_close_idle_connection_after_the_configured_keep_alive_timeout() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                traffic_shaping:
                    router:
                        keep_alive: 1s
                "#,
            )
            .build()
            .start()
            .await;

        let mut stream = TcpStream::connect(router.serv().addr())
            .await
            .expect("failed to open a raw TCP connection to the router");

        let first = try_request(&mut stream)
            .await
            .expect("expected a response to the first request on a fresh connection");
        assert!(
            first.starts_with(b"HTTP/1.1 200"),
            "expected 200 OK on the first request, got: {}",
            String::from_utf8_lossy(&first)
        );

        tokio::time::sleep(Duration::from_secs(3)).await;

        let second = try_request(&mut stream).await;
        assert!(
            second.is_none(),
            "expected the router to close the idle connection per the configured 1s \
             http.keep_alive, but it was still alive after 3s idle and answered: {:?}",
            second.map(|b| String::from_utf8_lossy(&b).to_string())
        );
    }

    #[ntex::test]
    async fn should_not_interrupt_a_slow_in_flight_request_with_a_short_keep_alive() {
        let subgraphs = TestSubgraphs::builder()
            .with_delay(Duration::from_secs(4))
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
                traffic_shaping:
                    router:
                        keep_alive: 2s
                "#,
            )
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request("{ topProducts { name price } }", None, None)
            .await;

        assert!(
            res.status().is_success(),
            "expected the 4s in-flight request to complete despite a 2s http.keep_alive, got status {}",
            res.status()
        );
        let json_body = res.json_body().await;
        assert!(
            json_body["data"]["topProducts"].is_array(),
            "expected a full GraphQL response, got: {json_body:?}"
        );
    }

    #[ntex::test]
    async fn should_disable_keep_alive_when_configured_to_zero() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                traffic_shaping:
                    router:
                        keep_alive: 0s
                "#,
            )
            .build()
            .start()
            .await;

        let mut stream = TcpStream::connect(router.serv().addr())
            .await
            .expect("failed to open a raw TCP connection to the router");

        let first = try_request(&mut stream)
            .await
            .expect("expected a response to the first request on a fresh connection");
        assert!(
            first.starts_with(b"HTTP/1.1 200"),
            "expected 200 OK on the first request, got: {}",
            String::from_utf8_lossy(&first)
        );

        // No idle wait: with keep-alive disabled, the connection should already be closing.
        let second = try_request(&mut stream).await;
        assert!(
            second.is_none(),
            "expected the connection to be closed immediately after the first response with \
             keep_alive: 0s, but it was still alive and answered: {:?}",
            second.map(|b| String::from_utf8_lossy(&b).to_string())
        );
    }

    /// The keep-alive timer must re-arm on every reuse, not just the first one: several
    /// successful reuse cycles under the configured timeout must not leave the connection
    /// either permanently alive or prematurely dead on a later cycle.
    #[ntex::test]
    async fn should_rearm_keep_alive_timer_across_multiple_reuse_cycles() {
        let router = TestRouter::builder()
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                traffic_shaping:
                    router:
                        keep_alive: 2s
                "#,
            )
            .build()
            .start()
            .await;

        let mut stream = TcpStream::connect(router.serv().addr())
            .await
            .expect("failed to open a raw TCP connection to the router");

        for cycle in 0..3 {
            let response = try_request(&mut stream).await.unwrap_or_else(|| {
                panic!("expected a response on reuse cycle {cycle}, but the connection was closed")
            });
            assert!(
                response.starts_with(b"HTTP/1.1 200"),
                "expected 200 OK on reuse cycle {cycle}, got: {}",
                String::from_utf8_lossy(&response)
            );

            // Idle well under the 2s keep_alive before reusing the connection again.
            tokio::time::sleep(Duration::from_millis(1200)).await;
        }

        // Idle past the 2s keep_alive: the connection must now actually be closed, proving the
        // timer still enforces the timeout after being re-armed multiple times.
        tokio::time::sleep(Duration::from_secs(2)).await;

        let after_timeout = try_request(&mut stream).await;
        assert!(
            after_timeout.is_none(),
            "expected the connection to finally be closed after an idle period exceeding the \
             2s keep_alive, but it was still alive and answered: {:?}",
            after_timeout.map(|b| String::from_utf8_lossy(&b).to_string())
        );
    }
}
