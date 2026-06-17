#[cfg(test)]
mod traffic_shaping_e2e_tests {
    use crate::testkit::TestRouter;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;

    #[ntex::test]
    async fn should_timeout_request_when_exceeding_router_request_timeout() {
        let router = TestRouter::builder()
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                traffic_shaping:
                    router:
                        request_timeout: 1s
                "#,
            )
            .build()
            .start()
            .await;

        let addr = router.serv().addr();
        let mut stream = TcpStream::connect(addr).await.expect("failed to connect");

        // Send the request headers using chunked transfer-encoding, followed by a single chunk that
        // holds only part of the GraphQL payload. We deliberately never send the terminating chunk,
        // so the router keeps waiting for the rest of the body until the 1s timeout fires.
        let partial = r#"{"query":"{ users"#;
        let request = format!(
            "POST /graphql HTTP/1.1\r\n\
             Host: {addr}\r\n\
             Content-Type: application/json\r\n\
             Accept: application/graphql-response+json\r\n\
             Transfer-Encoding: chunked\r\n\
             \r\n\
             {len:x}\r\n{partial}\r\n",
            len = partial.len(),
        );
        stream
            .write_all(request.as_bytes())
            .await
            .expect("failed to write request");
        stream.flush().await.expect("failed to flush");

        // Read the full response. The router responds after ~1s and then closes the connection,
        // so reading until EOF yields exactly the timeout response.
        let mut raw = Vec::new();
        let read = tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut raw))
            .await
            .expect("timed out waiting for the router response")
            .expect("failed to read response");
        assert!(read > 0, "expected a response from the router");

        let response = String::from_utf8(raw).expect("response was not valid UTF-8");
        let (status_line, body) = response
            .split_once("\r\n")
            .expect("malformed HTTP response");
        let body = body.rsplit("\r\n").next().unwrap_or_default();

        assert!(
            status_line.contains("504"),
            "expected 504 Gateway Timeout, got status line: {status_line:?}"
        );
        insta::assert_snapshot!(body, @r#"{"errors":[{"message":"Request timed out","extensions":{"code":"GATEWAY_TIMEOUT"}}]}"#);
    }

    /// Reproduces a user report: a keep-alive connection that idles longer than ntex's default 5s
    /// keep-alive gets closed by ntex itself, even though the router is configured with a higher
    /// `request_timeout`.
    ///
    // https://github.com/graphql-hive/router/issues/1144
    #[ntex::test]
    async fn should_timeout_using_router_not_ntex() {
        let router = TestRouter::builder()
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                traffic_shaping:
                    router:
                        request_timeout: 10s # ntex keep-alive defaults to 5s; a higher router timeout proves the router governs, not ntex
                "#,
            )
            .build()
            .start()
            .await;

        let addr = router.serv().addr();
        let mut stream = TcpStream::connect(addr).await.expect("failed to connect");

        // `{ __typename }` resolves to "Query" inside the router, so the POST /graphql path is
        // exercised without needing any subgraph running.
        let body = r#"{"query":"{ __typename }"}"#;
        let graphql_request = format!(
            "POST /graphql HTTP/1.1\r\n\
             Host: {addr}\r\n\
             Connection: keep-alive\r\n\
             Content-Type: application/json\r\n\
             Accept: application/graphql-response+json\r\n\
             Content-Length: {len}\r\n\
             \r\n\
             {body}",
            len = body.len(),
        );
        let mut buf = [0u8; 4096];

        // t=0: first request on the fresh keep-alive connection succeeds.
        stream
            .write_all(graphql_request.as_bytes())
            .await
            .expect("failed to write first request");
        stream.flush().await.expect("failed to flush first request");
        let n = stream
            .read(&mut buf)
            .await
            .expect("failed to read t=0 response");
        let first = String::from_utf8_lossy(&buf[..n]);
        assert!(first.contains("200"), "t=0 expected 200 OK, got: {first:?}");

        // Idle past ntex's 5s keep-alive default, but well within the router's 10s timeout.
        ntex::time::sleep(Duration::from_secs(6)).await;

        // t=6: the connection must still be usable. If ntex closed it at 5s, the write/read here
        // observes a closed socket (read returns 0 bytes or errors) — that is the bug we guard against.
        let _ = stream.write_all(graphql_request.as_bytes()).await;
        let _ = stream.flush().await;
        let n = stream
            .read(&mut buf)
            .await
            .expect("t=6 read failed: connection was closed by ntex before the router timeout");
        assert!(
            n > 0,
            "t=6 connection was closed by the server (ntex keep-alive fired before the router's request_timeout)"
        );
        let second = String::from_utf8_lossy(&buf[..n]);
        assert!(
            second.contains("200"),
            "t=6 expected 200 OK, got: {second:?}"
        );
    }
}
