#[cfg(test)]
mod read_deadline_tests {
    use std::time::Duration;

    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpStream,
        time::timeout,
    };

    use crate::testkit::{coprocessor::TestCoprocessor, ClientResponseExt, Started, TestRouter};

    async fn start_router() -> TestRouter<Started> {
        TestRouter::builder()
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                traffic_shaping:
                    router:
                        request_timeout: 30s
                        header_read_timeout: 1s
                        body_read_timeout: 1s
                telemetry:
                    metrics:
                        exporters:
                            - kind: prometheus
                "#,
            )
            .build()
            .start()
            .await
    }

    /// Writes `bytes` and never sends the rest of the request. Returns what the
    /// router sent back before closing the connection, or `None` if it was still
    /// waiting after 5s.
    async fn send_and_stall(router: &TestRouter<Started>, bytes: &[u8]) -> Option<Vec<u8>> {
        let mut stream = TcpStream::connect(router.serv().addr())
            .await
            .expect("failed to open a raw TCP connection to the router");
        stream
            .write_all(bytes)
            .await
            .expect("failed to write the partial request");

        let mut received = Vec::new();
        timeout(Duration::from_secs(5), async {
            let mut buf = [0u8; 4096];
            // read until EOF or a reset, either means the router gave up
            while let Ok(n @ 1..) = stream.read(&mut buf).await {
                received.extend_from_slice(&buf[..n]);
            }
        })
        .await
        .ok()
        .map(|_| received)
    }

    async fn scrape_metrics(router: &TestRouter<Started>) -> String {
        router
            .serv()
            .get("/metrics")
            .send()
            .await
            .expect("failed to scrape /metrics")
            .string_body()
            .await
    }

    /// Sums every series of the Prometheus metric `name` whose labels contain all
    /// of `labels`, so a value recorded twice under different labels still adds up.
    fn metric_sum(metrics: &str, name: &str, labels: &[&str]) -> f64 {
        metrics
            .lines()
            .filter_map(|line| line.strip_prefix(name)?.strip_prefix('{'))
            .filter(|series| labels.iter().all(|label| series.contains(label)))
            .filter_map(|series| series.rsplit_once(' ')?.1.parse::<f64>().ok())
            .sum()
    }

    /// Asserts the timeout was counted exactly once, and is the only GraphQL error.
    fn assert_counted_once(metrics: &str) {
        let errors_total = "hive_router_graphql_errors_total";
        assert_eq!(
            metric_sum(metrics, errors_total, &[r#"code="PAYLOAD_READ_TIMEOUT""#]),
            1.0,
            "expected {errors_total}{{code=\"PAYLOAD_READ_TIMEOUT\"}} to be 1, got:\n{metrics}"
        );
        assert_eq!(
            metric_sum(metrics, errors_total, &[]),
            1.0,
            "expected no other GraphQL errors, got:\n{metrics}"
        );
    }

    /// A client sends the request head with a `Content-Length` and never sends
    /// the body. Without a body read deadline, the router holds the socket and
    /// its buffers until `request_timeout`, and thousands of such requests OOM it.
    #[ntex::test]
    async fn should_reject_a_request_whose_body_never_arrives() {
        let router = start_router().await;

        let head = format!(
            "POST {} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: 64\r\n\r\n",
            router.graphql_path()
        );
        let received = send_and_stall(&router, head.as_bytes()).await.expect(
            "expected the router to give up on the missing body after the 1s body_read_timeout, \
             but it was still waiting after 5s",
        );

        let received = String::from_utf8_lossy(&received);
        assert!(
            received.starts_with("HTTP/1.1 408"),
            "expected 408 Request Timeout, got: {received}"
        );
        assert!(
            received.contains(r#""code":"PAYLOAD_READ_TIMEOUT""#),
            "expected the PAYLOAD_READ_TIMEOUT error code, got: {received}"
        );

        let metrics = scrape_metrics(&router).await;
        assert_counted_once(&metrics);
        assert_eq!(
            metric_sum(
                &metrics,
                "http_server_request_duration_seconds_count",
                &[r#"http_response_status_code="408""#]
            ),
            1.0,
            "expected a single 408 in http.server.request.duration, got:\n{metrics}"
        );
    }

    /// The coprocessor `router.request` stage with `include.body` reads the body
    /// before the pipeline does, and its error response skips the handler where
    /// pipeline errors are counted.
    #[ntex::test]
    async fn should_reject_and_count_a_missing_body_in_the_coprocessor_router_request_stage() {
        let mut coprocessor = TestCoprocessor::new().await;
        let host = coprocessor.host_with_port();
        // the body never arrives, so the coprocessor is never called
        let stage_mock = coprocessor.mock_stage("router.request").expect(0).create();

        let router = TestRouter::builder()
            .inline_config(format!(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                traffic_shaping:
                    router:
                        request_timeout: 30s
                        body_read_timeout: 1s
                coprocessor:
                    url: http://{host}/coprocessor
                    protocol: http1
                    stages:
                        router:
                            request:
                                include:
                                    body: true
                telemetry:
                    metrics:
                        exporters:
                            - kind: prometheus
                "#
            ))
            .build()
            .start()
            .await;

        let head = format!(
            "POST {} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: 64\r\n\r\n",
            router.graphql_path()
        );
        let received = send_and_stall(&router, head.as_bytes()).await.expect(
            "expected the coprocessor stage to give up on the missing body after the 1s \
             body_read_timeout, but it was still waiting after 5s",
        );

        let received = String::from_utf8_lossy(&received);
        assert!(
            received.starts_with("HTTP/1.1 408"),
            "expected 408 Request Timeout, got: {received}"
        );
        assert!(
            received.contains(r#""code":"PAYLOAD_READ_TIMEOUT""#),
            "expected the PAYLOAD_READ_TIMEOUT error code, got: {received}"
        );

        let metrics = scrape_metrics(&router).await;
        assert_counted_once(&metrics);

        stage_mock.assert_async().await;
    }

    /// A client opens a connection and never finishes the request head.
    #[ntex::test]
    async fn should_close_a_connection_whose_headers_never_complete() {
        let router = start_router().await;

        let partial_head = format!(
            "POST {} HTTP/1.1\r\nHost: localhost\r\n",
            router.graphql_path()
        );
        let received = send_and_stall(&router, partial_head.as_bytes()).await;

        assert!(
            received.is_some(),
            "expected the router to close the connection after the 1s header_read_timeout, \
             but it was still open after 5s"
        );
    }
}
