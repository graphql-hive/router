#[cfg(test)]
mod hive_cdn_supergraph_e2e_tests {
    use std::time::Duration;

    use ntex::{time, web::test};
    use sonic_rs::{from_slice, JsonContainerTrait, JsonValueTrait, Value};
    use tokio::time::sleep;

    use crate::testkit::{
        init_graphql_request, init_router_from_config_inline, wait_for_readiness, SubgraphsServer,
    };

    #[ntex::test]
    async fn should_load_supergraph_from_endpoint() {
        let subgraphs_server = SubgraphsServer::start().await;
        let mut server = mockito::Server::new_async().await;
        let host = server.host_with_port();

        let _mock = server
            .mock("GET", "/supergraph")
            .match_header("x-hive-cdn-key", "dummy_key")
            .with_status(200)
            .with_header("content-type", "text/plain")
            .with_body(include_str!("../supergraph.graphql"))
            .create();

        let app = init_router_from_config_inline(&format!(
            r#"supergraph:
                source: hive
                endpoint: http://{host}/supergraph
                key: dummy_key
          "#,
        ))
        .await
        .expect("failed to start router");

        wait_for_readiness(&app.app).await;

        let resp = test::call_service(
            &app.app,
            init_graphql_request("{ users { id } }", None).to_request(),
        )
        .await;

        assert!(resp.status().is_success(), "Expected 200 OK");

        let subgraph_requests = subgraphs_server
            .get_subgraph_requests_log("accounts")
            .await
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

        let app = init_router_from_config_inline(&format!(
            r#"supergraph:
              source: hive
              endpoint: http://{host}/supergraph
              key: dummy_key
              poll_interval: 100ms
        "#,
        ))
        .await
        .expect("failed to start router");

        wait_for_readiness(&app.app).await;

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

        let app = init_router_from_config_inline(&format!(
            r#"supergraph:
              source: hive
              endpoint: http://{host}/supergraph
              key: dummy_key
              poll_interval: 100ms
        "#,
        ))
        .await
        .expect("failed to start router");

        wait_for_readiness(&app.app).await;
        mock1.assert();
        sleep(Duration::from_millis(150)).await;
        mock2.assert();
        sleep(Duration::from_millis(150)).await;
        mock3.assert();
        sleep(Duration::from_millis(150)).await;
        mock4.assert();

        let resp = test::call_service(
            &app.app,
            init_graphql_request("{ __schema { types { name } } }", None).to_request(),
        )
        .await;

        // make sure schema now loaded and has new types
        assert!(resp.status().is_success(), "Expected 200 OK");
        let json_body: Value = from_slice(&test::read_body(resp).await).unwrap();
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

        let app = init_router_from_config_inline(&format!(
            r#"supergraph:
              source: hive
              endpoint: http://{host}/supergraph
              key: dummy_key
              poll_interval: 800ms
        "#,
        ))
        .await
        .expect("failed to start router");

        wait_for_readiness(&app.app).await;
        mock1.assert();

        let resp = test::call_service(
            &app.app,
            init_graphql_request("{ __schema { types { name } } }", None).to_request(),
        )
        .await;

        assert!(resp.status().is_success(), "Expected 200 OK");

        let json_body: Value = from_slice(&test::read_body(resp).await).unwrap();
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

        let resp = test::call_service(
            &app.app,
            init_graphql_request("{ __schema { types { name } } }", None).to_request(),
        )
        .await;

        assert!(resp.status().is_success(), "Expected 200 OK");
        let json_body: Value = from_slice(&test::read_body(resp).await).unwrap();
        let types_arr = json_body
            .get("data")
            .unwrap()
            .get("__schema")
            .unwrap()
            .get("types")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(types_arr.len(), 17);
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

        let app = init_router_from_config_inline(&format!(
            r#"supergraph:
              source: hive
              endpoint: http://{host}/supergraph
              key: dummy_key
              request_timeout: 100ms
              retry_policy:
                max_retries: 10
        "#,
        ))
        .await
        .expect("failed to start router");

        wait_for_readiness(&app.app).await;
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

        let app = init_router_from_config_inline(&format!(
            r#"supergraph:
              source: hive
              endpoint: http://{host}/supergraph
              key: dummy_key
              poll_interval: 100ms
              retry_policy:
                max_retries: 3
        "#,
        ))
        .await
        .expect("failed to start router");

        time::sleep(Duration::from_secs(7)).await;

        // Health should be ok, readiness not
        let req = test::TestRequest::get().uri("/health").to_request();
        let response = app.call(req).await.expect("failed to check health");
        assert!(response.status().is_success());
        let req = test::TestRequest::get().uri("/readiness").to_request();
        let response = app.call(req).await.expect("failed to check readiness");
        assert!(response.status().is_server_error());

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

        let app = init_router_from_config_inline(&format!(
            r#"supergraph:
              source: hive
              endpoint:
                - http://{host1}/supergraph
                - http://{host2}/supergraph
              key: dummy_key
              retry_policy:
                max_retries: 1
        "#,
        ))
        .await
        .expect("failed to start router");

        wait_for_readiness(&app.app).await;

        let resp = test::call_service(
            &app.app,
            init_graphql_request("{ __type(name: \"Product\") { name } }", None).to_request(),
        )
        .await;

        assert!(resp.status().is_success(), "Expected 200 OK");
        let json_body: Value =
            from_slice(&test::read_body(resp).await).expect("failed to read body");
        let json_str = sonic_rs::to_string_pretty(&json_body).expect("bad response");
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
