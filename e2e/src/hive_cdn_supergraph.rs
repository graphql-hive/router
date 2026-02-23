#[cfg(test)]
mod hive_cdn_supergraph_e2e_tests {
    use std::time::Duration;

    use ntex::time;
    use sonic_rs::{from_slice, JsonContainerTrait, JsonValueTrait, Value};
    use tokio::time::sleep;

    use crate::testkit_v2::{TestRouterBuilder, TestSubgraphsBuilder};

    #[ntex::test]
    async fn should_load_supergraph_from_endpoint() {
        let subgraphs = TestSubgraphsBuilder::new().build().start().await;

        let mut server = mockito::Server::new_async().await;
        let host = server.host_with_port();

        let _mock = server
            .mock("GET", "/supergraph")
            .match_header("x-hive-cdn-key", "dummy_key")
            .with_status(200)
            .with_header("content-type", "text/plain")
            .with_body(subgraphs.supergraph_with_addr(include_str!("../supergraph.graphql")))
            .create();

        let router = TestRouterBuilder::new()
            .inline_config(&format!(
                r#"
                supergraph:
                    source: hive
                    endpoint: http://{host}/supergraph
                    key: dummy_key
                "#,
            ))
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request("{ users { id } }", None, None)
            .await;

        assert!(res.status().is_success(), "Expected 200 OK");

        let subgraph_requests = subgraphs
            .get_requests_log("accounts")
            .expect("expected requests sent to accounts subgraph");
        assert_eq!(
            subgraph_requests.len(),
            1,
            "expected 1 request to accounts subgraph"
        );
    }

    #[ntex::test]
    async fn should_use_etag_for_caching() {
        let mut server = mockito::Server::new_async().await;
        let host = server.host_with_port();
        let _mock1 = server
            .mock("GET", "/supergraph")
            .expect(1)
            .match_header("x-hive-cdn-key", "dummy_key")
            .with_status(200)
            .with_header("content-type", "text/plain")
            .with_header("etag", "dummy_etag")
            .with_body("type Query { dummy: String }")
            .create();

        let mock2 = server
            .mock("GET", "/supergraph")
            .expect_at_least(3)
            .match_header("x-hive-cdn-key", "dummy_key")
            .match_header("if-none-match", "dummy_etag")
            .with_status(304)
            .create();

        let _router = TestRouterBuilder::new()
            .inline_config(&format!(
                r#"
                supergraph:
                    source: hive
                    endpoint: http://{host}/supergraph
                    key: dummy_key
                    poll_interval: 100ms
                "#,
            ))
            .build()
            .start()
            .await;

        sleep(Duration::from_millis(500)).await;
        mock2.assert();
    }

    #[ntex::test]
    async fn should_use_etag_for_caching_and_should_swap_correctly() {
        let mut server = mockito::Server::new_async().await;
        let host = server.host_with_port();
        let mock1 = server
            .mock("GET", "/supergraph")
            .expect(1)
            .match_header("x-hive-cdn-key", "dummy_key")
            .with_status(200)
            .with_header("content-type", "text/plain")
            .with_header("etag", "dummy_etag")
            .with_body("type Query { dummy: String }")
            .create();

        let mock2 = server
            .mock("GET", "/supergraph")
            .expect(1)
            .match_header("x-hive-cdn-key", "dummy_key")
            .match_header("if-none-match", "dummy_etag")
            .with_status(304)
            .create();

        let mock3 = server
            .mock("GET", "/supergraph")
            .expect_at_least(1)
            .match_header("x-hive-cdn-key", "dummy_key")
            .match_header("if-none-match", "dummy_etag")
            .with_status(200)
            .with_header("content-type", "text/plain")
            .with_header("etag", "dummy_etag_new")
            .with_body("type Query { dummyNew: NewType } type NewType { id: ID! }")
            .create();

        let mock4 = server
            .mock("GET", "/supergraph")
            .expect_at_least(1)
            .match_header("x-hive-cdn-key", "dummy_key")
            .match_header("if-none-match", "dummy_etag_new")
            .with_status(304)
            .create();

        let router = TestRouterBuilder::new()
            .inline_config(&format!(
                r#"
                supergraph:
                    source: hive
                    endpoint: http://{host}/supergraph
                    key: dummy_key
                    poll_interval: 100ms
                "#,
            ))
            .build()
            .start()
            .await;

        mock1.assert();
        sleep(Duration::from_millis(150)).await;
        mock2.assert();
        sleep(Duration::from_millis(150)).await;
        mock3.assert();
        sleep(Duration::from_millis(150)).await;
        mock4.assert();

        let res = router
            .send_graphql_request("{ __schema { types { name } } }", None, None)
            .await;

        // make sure schema now loaded and has new types
        assert!(res.status().is_success(), "Expected 200 OK");
        let json_body: Value = from_slice(&res.body().await.unwrap()).unwrap();
        let types_arr = json_body
            .get("data")
            .unwrap()
            .get("__schema")
            .unwrap()
            .get("types")
            .unwrap();
        assert!(sonic_rs::to_string(types_arr)
            .expect("bad resonse")
            .contains("NewType"));
    }

    #[ntex::test]
    async fn should_reload_supergraph_from_endpoint() {
        let mut server = mockito::Server::new_async().await;
        let host = server.host_with_port();

        let mock1 = server
            .mock("GET", "/supergraph")
            .expect(1)
            .match_header("x-hive-cdn-key", "dummy_key")
            .with_status(200)
            .with_header("content-type", "text/plain")
            .with_body("type Query { dummy: String }")
            .create();

        let mock2 = server
            .mock("GET", "/supergraph")
            .expect_at_least(1)
            .match_header("x-hive-cdn-key", "dummy_key")
            .with_status(200)
            .with_header("content-type", "text/plain")
            .with_body(include_str!("../supergraph.graphql"))
            .create();

        let router = TestRouterBuilder::new()
            .inline_config(&format!(
                r#"
                supergraph:
                    source: hive
                    endpoint: http://{host}/supergraph
                    key: dummy_key
                    poll_interval: 800ms
                "#,
            ))
            .build()
            .start()
            .await;

        mock1.assert();

        let res = router
            .send_graphql_request("{ __schema { types { name } } }", None, None)
            .await;

        assert!(res.status().is_success(), "Expected 200 OK");

        let json_body: Value = from_slice(&res.body().await.unwrap()).unwrap();
        let types_arr = json_body
            .get("data")
            .unwrap()
            .get("__schema")
            .unwrap()
            .get("types")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(types_arr.len(), 14);

        // Now wait for the schema to be reloaded and updated
        sleep(Duration::from_millis(900)).await;
        mock2.assert();

        let res = router
            .send_graphql_request("{ __schema { types { name } } }", None, None)
            .await;

        assert!(res.status().is_success(), "Expected 200 OK");
        let json_body: Value = from_slice(&res.body().await.unwrap()).unwrap();
        let types_arr = json_body
            .get("data")
            .unwrap()
            .get("__schema")
            .unwrap()
            .get("types")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(types_arr.len(), 18);
    }

    #[ntex::test]
    async fn should_handle_failures_with_retry() {
        let mut server = mockito::Server::new_async().await;
        let host = server.host_with_port();

        let one = server
            .mock("GET", "/supergraph")
            .expect(3)
            .with_status(500)
            .create();

        let two = server
            .mock("GET", "/supergraph")
            .expect_at_least(1)
            .with_status(200)
            .with_header("content-type", "text/plain")
            .with_body("type Query { dummy: String }")
            .create();

        let router = TestRouterBuilder::new()
            .inline_config(&format!(
                r#"
                supergraph:
                    source: hive
                    endpoint: http://{host}/supergraph
                    key: dummy_key
                    request_timeout: 100ms
                    retry_policy:
                        max_retries: 10
                "#,
            ))
            .without_wait_for_ready() // this one will time out after 3 seconds, we need more
            .build()
            .start()
            .await;

        tokio::time::timeout(Duration::from_secs(7), async {
            loop {
                match router.serv().get("/readiness").send().await {
                    Ok(response) => {
                        if response.status() == 200 {
                            break;
                        }
                    }
                    Err(_) => {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                }
            }
        })
        .await
        .expect("/readiness did not return 200 within 7 seconds");

        one.assert();
        two.assert();
    }

    #[ntex::test]
    async fn should_fail_when_eventually_cant_load() {
        let mut server = mockito::Server::new_async().await;
        let host = server.host_with_port();
        let mock = server
            .mock("GET", "/supergraph")
            .expect_at_least(5)
            .with_status(500)
            .create();

        let router = TestRouterBuilder::new()
            .inline_config(&format!(
                r#"
                supergraph:
                    source: hive
                    endpoint: http://{host}/supergraph
                    key: dummy_key
                    poll_interval: 100ms
                    retry_policy:
                        max_retries: 3
                "#,
            ))
            .without_wait_for_health()
            .without_wait_for_ready()
            .build()
            .start()
            .await;

        // TODO: waiting for 7 seconds is hella long
        time::sleep(Duration::from_secs(7)).await;

        // Health should be ok,
        let res = router.serv().get("/health").send().await.unwrap();
        assert!(res.status().is_success());

        // readiness not
        let res = router.serv().get("/readiness").send().await.unwrap();
        assert!(res.status().is_server_error());

        mock.assert();
    }

    #[ntex::test]
    async fn should_support_multiple_endpoints() {
        let mut server1 = mockito::Server::new_async().await;

        // Failing server so it will try the next one
        let host1 = server1.host_with_port();
        let mock1 = server1
            .mock("GET", "/supergraph")
            // 1 first attempt, 1 retry
            .expect(2)
            .match_header("x-hive-cdn-key", "dummy_key")
            .with_status(500)
            .create();

        let mut server2 = mockito::Server::new_async().await;
        let host2 = server2.host_with_port();
        let mock2 = server2
            .mock("GET", "/supergraph")
            .expect(1)
            .match_header("x-hive-cdn-key", "dummy_key")
            .with_status(200)
            .with_header("content-type", "text/plain")
            .with_body(include_str!("../supergraph.graphql"))
            .create();

        let router = TestRouterBuilder::new()
            .inline_config(&format!(
                r#"
                supergraph:
                    source: hive
                    endpoint:
                        - http://{host1}/supergraph
                        - http://{host2}/supergraph
                    key: dummy_key
                    retry_policy:
                        max_retries: 1
                "#,
            ))
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request("{ __type(name: \"Product\") { name } }", None, None)
            .await;

        assert!(res.status().is_success(), "Expected 200 OK");
        let json_body: Value = from_slice(&res.body().await.unwrap()).expect("failed to read body");
        let json_str = sonic_rs::to_string_pretty(&json_body).expect("bad resonse");
        insta::assert_snapshot!(json_str, @r#"
        {
          "data": {
            "__type": {
              "name": "Product"
            }
          }
        }
        "#);

        mock1.assert();
        mock2.assert();
    }
}
