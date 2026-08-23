#[cfg(test)]
mod keep_alive_tests {
    use std::time::Duration;

    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpStream,
        time::timeout,
    };

    use crate::testkit::{TestRouter, TestSubgraphs};

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
                http:
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
}
