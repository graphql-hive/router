#[cfg(test)]
mod hive_cdn_supergraph_e2e_tests {
    use std::time::Duration;

    use hive_router_config::supergraph::SupergraphSource;
    use hive_router_config::{load_config, primitives::single_or_multiple::SingleOrMultiple};
    use ntex::time;
    use sonic_rs::{JsonContainerTrait, JsonValueTrait};
    use tokio::time::sleep;

    use crate::testkit::{
        wait_until_mock_matched, ClientResponseExt, EnvVarsGuard, TestRouter, TestSubgraphs,
    };

    #[ntex::test]
    async fn should_load_supergraph_from_endpoint() {
        let subgraphs = TestSubgraphs::builder().build().start().await;

        let mut server = mockito::Server::new_async().await;
        let host = server.host_with_port();

        let _mock = server
            .mock("GET", "/supergraph")
            .match_header("x-hive-cdn-key", "dummy_key")
            .with_status(200)
            .with_header("content-type", "text/plain")
            .with_body(subgraphs.supergraph(include_str!("../supergraph.graphql")))
            .create();

        let router = TestRouter::builder()
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

        let _router = TestRouter::builder()
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

        let router = TestRouter::builder()
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
        let json_body = res.json_body().await;
        let types_arr = json_body
            .get("data")
            .unwrap()
            .get("__schema")
            .unwrap()
            .get("types")
            .unwrap();
        assert!(sonic_rs::to_string(types_arr)
            .expect("bad response")
            .contains("NewType"));
    }

    #[ntex::test]
    async fn should_reload_supergraph_from_endpoint() {
        let mut server = mockito::Server::new_async().await;
        let host = server.host_with_port();

        let mock1 = server
            .mock("GET", "/supergraph")
            .expect_at_least(1)
            .match_header("x-hive-cdn-key", "dummy_key")
            .with_status(200)
            .with_header("content-type", "text/plain")
            .with_body("type Query { dummy: String }")
            .create();

        let router = TestRouter::builder()
            .inline_config(&format!(
                r#"
                supergraph:
                    source: hive
                    endpoint: http://{host}/supergraph
                    key: dummy_key
                    poll_interval: 500ms
                "#,
            ))
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request("{ __schema { types { name } } }", None, None)
            .await;

        assert!(res.status().is_success(), "Expected 200 OK");

        let json_body = res.json_body().await;
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

        // Remove first mock and register the new supergraph mock
        mock1.remove();
        let mock2 = server
            .mock("GET", "/supergraph")
            .expect_at_least(1)
            .match_header("x-hive-cdn-key", "dummy_key")
            .with_status(200)
            .with_header("content-type", "text/plain")
            .with_body(include_str!("../supergraph.graphql"))
            .create();

        // Wait for the poller to pick up the new supergraph
        wait_until_mock_matched(&mock2)
            .await
            .expect("Expected mock2 to be matched");

        let res = router
            .send_graphql_request("{ __schema { types { name } } }", None, None)
            .await;

        assert!(res.status().is_success(), "Expected 200 OK");
        let json_body = res.json_body().await;
        let types_arr = json_body
            .get("data")
            .unwrap()
            .get("__schema")
            .unwrap()
            .get("types")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(types_arr.len(), 22);
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

        let router = TestRouter::builder()
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
            .build()
            .start()
            .await;

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

        let router = TestRouter::builder()
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
            .skip_wait_for_healthy_on_start()
            .skip_wait_for_ready_on_start()
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

        let router = TestRouter::builder()
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
        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
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

    #[ntex::test]
    async fn should_support_multiple_endpoints_from_env() {
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

        let endpoints = vec![
            format!("http://{host1}/supergraph"),
            format!("http://{host2}/supergraph"),
        ];

        let _env_guard = EnvVarsGuard::new()
            .set("HIVE_CDN_ENDPOINT", &endpoints.join(","))
            .set("HIVE_CDN_KEY", "dummy_key")
            .apply()
            .await;

        let router = TestRouter::builder();

        let mut config = load_config(None)
            .expect("failed to load router config from env with multiple endpoints");

        match &mut config.supergraph {
            SupergraphSource::HiveConsole {
                endpoint: Some(SingleOrMultiple::Multiple(eps)),
                retry_policy,
                ..
            } => {
                assert_eq!(eps, &endpoints);
                retry_policy.max_retries = 1; // set retries to 1 for the test
            }
            _ => panic!(
                "Expected supergraph source to be Hive Console with multiple endpoints, got: {:#?}",
                config.supergraph
            ),
        }

        let router = router.set_config(config).build().start().await;

        let res = router
            .send_graphql_request("{ __type(name: \"Product\") { name } }", None, None)
            .await;

        assert!(res.status().is_success(), "Expected 200 OK");
        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
        {
          "data": {
            "__type": {
              "name": "Product"
            }
          }
        }
        "#);

        mock1.assert_async().await;
        mock2.assert_async().await;
    }
}
