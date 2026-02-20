#[cfg(test)]
mod override_subgraph_urls_e2e_tests {
    use sonic_rs::{json, Value};

    use crate::testkit_v2::{some_header_map, TestRouterBuilder, TestSubgraphsBuilder};

    #[ntex::test]
    async fn should_not_deadlock_when_overriding_subgraph_timeout_statically() {
        let subgraphs = TestSubgraphsBuilder::new().build().start().await;
        let router = TestRouterBuilder::new()
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
            let body = res.body().await.unwrap();
            let json_body: Value = sonic_rs::from_slice(&body).unwrap();
            assert_eq!(json_body, expected_json);
        }
    }

    #[ntex::test]
    async fn should_not_deadlock_when_overriding_subgraph_timeout_dynamically() {
        let subgraphs = TestSubgraphsBuilder::new().build().start().await;
        let router = TestRouterBuilder::new()
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
            let body = res.body().await.unwrap();
            let json_body: Value = sonic_rs::from_slice(&body).unwrap();
            assert_eq!(json_body, expected_json);
        }
    }
}
