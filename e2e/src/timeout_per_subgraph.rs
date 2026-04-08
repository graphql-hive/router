#[cfg(test)]
mod timeout_per_subgraph_e2e_tests {
    use std::{thread::sleep, time::Duration};

    use ntex::http::StatusCode;
    use sonic_rs::json;

    use crate::testkit::{some_header_map, ClientResponseExt, TestRouter, TestSubgraphs};

    #[ntex::test]
    async fn should_apply_static_subgraph_timeout_override() {
        let subgraphs = TestSubgraphs::builder()
            .with_on_request(|request| {
                if request.path == "/accounts" {
                    sleep(Duration::from_secs(3));
                }
                None
            })
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
            .with_on_request(|request| {
                if request.path == "/accounts" {
                    sleep(Duration::from_secs(3));
                }
                None
            })
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
                      "message": "Request to subgraph timed out after 2000 milliseconds",
                      "extensions": {
                        "code": "SUBGRAPH_REQUEST_TIMEOUT",
                        "serviceName": "accounts"
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
            .file_config("configs/timeout_per_subgraph_dynamic.router.yaml")
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
}
