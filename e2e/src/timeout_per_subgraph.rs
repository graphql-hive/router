#[cfg(test)]
mod timeout_per_subgraph_e2e_tests {
    use std::time::Duration;

    use ntex::http::StatusCode;
    use sonic_rs::json;

    use crate::testkit::{some_header_map, ClientResponseExt, TestRouter, TestSubgraphs};

    const SUBGRAPH_DELAY: Duration = Duration::from_millis(300);

    #[ntex::test]
    async fn should_apply_static_subgraph_timeout_override() {
        let subgraphs = TestSubgraphs::builder()
            .with_delay(SUBGRAPH_DELAY)
            .build()
            .start()
            .await;

        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .file_config("configs/timeout_per_subgraph_static.router.yaml")
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request("{ users { id } }", None, None)
            .await;

        assert_eq!(res.status(), StatusCode::OK, "Expected 200 OK");
        let expected_json = json!({
          "data": {
            "users": [
              { "id": "1" },
              { "id": "2" },
              { "id": "3" },
              { "id": "4" },
              { "id": "5" },
              { "id": "6" }
            ]
          }
        });

        let json_body = res.json_body().await;
        assert_eq!(json_body, expected_json);
    }

    #[ntex::test]
    async fn should_apply_dynamic_subgraph_timeout_override_and_fallback_to_default() {
        let subgraphs = TestSubgraphs::builder()
            .with_delay(SUBGRAPH_DELAY)
            .build()
            .start()
            .await;

        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .file_config("configs/timeout_per_subgraph_dynamic.router.yaml")
            .build()
            .start()
            .await;

        let short_timeout_res = router
            .send_graphql_request(
                "{ users { id } }",
                None,
                some_header_map! {
                        http::header::HeaderName::from_static("x-timeout") => "short"
                },
            )
            .await;

        assert_eq!(
            short_timeout_res.status(),
            StatusCode::OK,
            "Expected 200 OK"
        );

        let expected_json = json!({
          "data": {
            "users": [
              { "id": "1" },
              { "id": "2" },
              { "id": "3" },
              { "id": "4" },
              { "id": "5" },
              { "id": "6" }
            ]
          }
        });

        let json_body = short_timeout_res.json_body().await;
        assert_eq!(json_body, expected_json);

        let default_timeout_res = router
            .send_graphql_request(
                "{ users { id } }",
                None,
                some_header_map! {
                        http::header::HeaderName::from_static("x-timeout") => "long"
                },
            )
            .await;

        assert_eq!(
            default_timeout_res.status(),
            StatusCode::OK,
            "Expected 200 OK"
        );
        insta::assert_snapshot!(
                default_timeout_res.json_body_string_pretty().await,
                @r#"
                {
                  "data": {
                    "users": null
                  },
                  "errors": [
                    {
                      "message": "Request to subgraph timed out",
                      "extensions": {
                        "code": "SUBGRAPH_REQUEST_TIMEOUT",
                        "service": "accounts"
                      }
                    }
                  ]
                }
                "#
        );
    }

    #[ntex::test]
    async fn should_not_deadlock_when_overriding_subgraph_timeout_statically() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .file_config("configs/timeout_per_subgraph_static.router.yaml")
            .build()
            .start()
            .await;

        let (res1, res2, res3, res4) = tokio::join!(
            router.send_graphql_request("{ users { id } }", None, None),
            router.send_graphql_request("{ users { id } }", None, None),
            router.send_graphql_request("{ users { id } }", None, None),
            router.send_graphql_request("{ users { id } }", None, None),
        );

        assert!(res1.status().is_success(), "Expected 200 OK");
        assert!(res2.status().is_success(), "Expected 200 OK");
        assert!(res3.status().is_success(), "Expected 200 OK");
        assert!(res4.status().is_success(), "Expected 200 OK");

        let expected_json = json!({
          "data": {
            "users": [
              { "id": "1" },
              { "id": "2" },
              { "id": "3" },
              { "id": "4" },
              { "id": "5" },
              { "id": "6" }
            ]
          }
        });

        for res in [res1, res2, res3, res4] {
            let json_body = res.json_body().await;
            assert_eq!(json_body, expected_json);
        }
    }

    #[ntex::test]
    async fn should_not_deadlock_when_overriding_subgraph_timeout_dynamically() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .file_config("configs/timeout_per_subgraph_dynamic_deadlock.router.yaml")
            .build()
            .start()
            .await;

        // We want to ensure that concurrent requests with different timeout settings
        // do not cause deadlocks. We are not testing the actual timeout duration here.
        let (res1, res2, res3, res4) = tokio::join!(
            router.send_graphql_request(
                "{ users { id } }",
                None,
                some_header_map! {
                    http::header::HeaderName::from_static("x-timeout") => "short"
                }
            ),
            router.send_graphql_request(
                "{ users { id } }",
                None,
                some_header_map! {
                    http::header::HeaderName::from_static("x-timeout") => "long"
                }
            ),
            router.send_graphql_request(
                "{ users { id } }",
                None,
                some_header_map! {
                    http::header::HeaderName::from_static("x-timeout") => "short"
                }
            ),
            router.send_graphql_request(
                "{ users { id } }",
                None,
                some_header_map! {
                    http::header::HeaderName::from_static("x-timeout") => "long"
                }
            ),
        );

        assert!(res1.status().is_success(), "Expected 200 OK");
        assert!(res2.status().is_success(), "Expected 200 OK");
        assert!(res3.status().is_success(), "Expected 200 OK");
        assert!(res4.status().is_success(), "Expected 200 OK");

        let expected_json = json!({
          "data": {
            "users": [
              { "id": "1" },
              { "id": "2" },
              { "id": "3" },
              { "id": "4" },
              { "id": "5" },
              { "id": "6" }
            ]
          }
        });

        for res in [res1, res2, res3, res4] {
            let json_body = res.json_body().await;
            assert_eq!(json_body, expected_json);
        }
    }

    /// A subgraph that sends its headers and part of the body, then stops sending.
    fn start_stalling_subgraph() -> String {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = stream.unwrap();
                std::thread::spawn(move || {
                    let mut buf = [0u8; 8192];
                    let _ = stream.read(&mut buf);
                    let _ = stream.write_all(
                        b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 100\r\n\r\n{\"data\":",
                    );
                    let _ = stream.flush();
                    // keep the connection open without sending the rest
                    std::thread::sleep(Duration::from_secs(30));
                });
            }
        });
        format!("http://{addr}/accounts")
    }

    #[ntex::test]
    async fn should_time_out_when_subgraph_stalls_mid_body() {
        let url = start_stalling_subgraph();
        let router = TestRouter::builder()
            .inline_config(&format!(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                error_masking:
                  all:
                    enabled: false
                traffic_shaping:
                    all:
                        request_timeout: 300ms
                override_subgraph_urls:
                    subgraphs:
                        accounts:
                            url: "{url}"
                "#
            ))
            .build()
            .start()
            .await;

        let started = std::time::Instant::now();
        let res = router
            .send_graphql_request("{ users { id } }", None, None)
            .await;
        let body = res.json_body_string_pretty().await;

        assert!(
            started.elapsed() < Duration::from_secs(5),
            "took {:?}",
            started.elapsed()
        );
        assert!(body.contains("SUBGRAPH_REQUEST_TIMEOUT"), "{body}");
    }
}
