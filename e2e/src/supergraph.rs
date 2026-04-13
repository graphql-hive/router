#[cfg(test)]
mod supergraph_e2e_tests {
    use std::time::Duration;

    use sonic_rs::JsonValueTrait;

    use crate::testkit::{wait_until_mock_matched, ClientResponseExt, TestRouter, TestSubgraphs};

    #[ntex::test]
    async fn should_clear_internal_caches_when_supergraph_changes() {
        let mut server = mockito::Server::new_async().await;
        let host = server.host_with_port();
        let mock1 = server
            .mock("GET", "/supergraph")
            .expect_at_least(1)
            .with_status(200)
            .with_header("content-type", "text/plain")
            .with_body("type Query { dummy: String }")
            .create();

        let router = TestRouter::builder()
            .inline_config(format!(
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

        // wait for caches to populate
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            router
                .schema_state()
                .normalize_cache
                .run_pending_tasks()
                .await;
            router.schema_state().plan_cache.run_pending_tasks().await;
            if router.schema_state().plan_cache.entry_count() >= 1
                && router.schema_state().normalize_cache.entry_count() >= 1
            {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for caches to populate: plan={}, normalize={}",
                router.schema_state().plan_cache.entry_count(),
                router.schema_state().normalize_cache.entry_count()
            );
            ntex::time::sleep(Duration::from_millis(50)).await;
        }

        mock1.remove();
        let mock2 = server
            .mock("GET", "/supergraph")
            .expect_at_least(1)
            .with_status(200)
            .with_header("content-type", "text/plain")
            .with_body("type Query { dummyNew: NewType } type NewType { id: ID! }")
            .create();

        wait_until_mock_matched(&mock2)
            .await
            .expect("Expected mock2 to be matched");

        // wait for the router to finish rebuilding with the new supergraph
        router.wait_for_ready(None).await;

        // wait for cache invalidation to be reflected (moka invalidate_all is lazy)
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            router
                .schema_state()
                .normalize_cache
                .run_pending_tasks()
                .await;
            router.schema_state().plan_cache.run_pending_tasks().await;
            if router.schema_state().plan_cache.entry_count() == 0
                && router.schema_state().normalize_cache.entry_count() == 0
            {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for caches to clear: plan={}, normalize={}",
                router.schema_state().plan_cache.entry_count(),
                router.schema_state().normalize_cache.entry_count()
            );
            ntex::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// In this test we are testing that the supergraph is not changed for in-flight requests.
    ///
    /// To do that, we are running the following timeline flow:
    ///
    /// 1. Start the server with Supergraph that has multiple subgraphs.
    /// 2. Run a request that queries data from A and then B (request B depends on B).
    /// 3. Subgraphs are set to a fixed delay of 500ms.
    /// 4. Then, while request to subgraph A is in flight, we reload and change the supergraph to a new one.
    /// 5. The request to subgraph A and B should still use the old supergraph and the old state.
    /// 6. New request should use the new supergraph and new state, so running the same query should fail now with a validation error.
    #[ntex::test]
    async fn should_not_change_supergraph_for_in_flight_requests() {
        let subgraphs = TestSubgraphs::builder()
            .with_delay(Duration::from_millis(500))
            .build()
            .start()
            .await;

        let mut server = mockito::Server::new_async().await;
        let host = server.host_with_port();

        // First supergraph
        let supergraph1_sdl = subgraphs.supergraph(include_str!("../supergraph.graphql"));
        let mock1 = server
            .mock("GET", "/supergraph")
            .expect_at_least(1)
            .with_status(200)
            .with_header("content-type", "text/plain")
            .with_header("etag", "1")
            .with_body(supergraph1_sdl)
            .create();

        let router = TestRouter::builder()
            .inline_config(format!(
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

        let res = router
            .send_graphql_request("{ users { id name reviews { id body } } }", None, None)
            .await;

        assert!(res.status().is_success(), "Expected 200 OK");

        let body_json = res.json_body().await;

        assert!(body_json["data"].is_object());
        assert!(body_json["errors"].is_null());

        mock1.remove();

        // Second supergraph - only registered after the first request completes so the poller
        // cannot swap the schema while the first request is in flight
        let supergraph2_sdl = subgraphs.supergraph(
            r#"schema
                  @link(url: "https://specs.apollo.dev/link/v1.0")
                  @link(url: "https://specs.apollo.dev/join/v0.3", for: EXECUTION) {
                  query: Query
                }

                directive @join__enumValue(graph: join__Graph!) repeatable on ENUM_VALUE

                directive @join__field(
                  graph: join__Graph
                  requires: join__FieldSet
                  provides: join__FieldSet
                  type: String
                  external: Boolean
                  override: String
                  usedOverridden: Boolean
                ) repeatable on FIELD_DEFINITION | INPUT_FIELD_DEFINITION

                directive @join__graph(name: String!, url: String!) on ENUM_VALUE

                directive @join__implements(
                  graph: join__Graph!
                  interface: String!
                ) repeatable on OBJECT | INTERFACE

                directive @join__type(
                  graph: join__Graph!
                  key: join__FieldSet
                  extension: Boolean! = false
                  resolvable: Boolean! = true
                  isInterfaceObject: Boolean! = false
                ) repeatable on OBJECT | INTERFACE | UNION | ENUM | INPUT_OBJECT | SCALAR

                directive @join__unionMember(
                  graph: join__Graph!
                  member: String!
                ) repeatable on UNION

                directive @link(
                  url: String
                  as: String
                  for: link__Purpose
                  import: [link__Import]
                ) repeatable on SCHEMA

                scalar join__FieldSet

                enum join__Graph {
                  PRODUCTS @join__graph(name: "products", url: "http://0.0.0.0:4200/products")
                }

                scalar link__Import

                enum link__Purpose {
                  """
                  `SECURITY` features provide metadata necessary to securely resolve fields.
                  """
                  SECURITY

                  """
                  `EXECUTION` features provide metadata necessary for operation execution.
                  """
                  EXECUTION
                }

                type Product
                  @join__type(graph: PRODUCTS, key: "upc") {
                  upc: String!
                  name: String @join__field(graph: PRODUCTS)
                }

                type Query
                  @join__type(graph: PRODUCTS){
                  topProducts(first: Int = 5): [Product] @join__field(graph: PRODUCTS)
                }
            "#,
        );
        let mock2 = server
            .mock("GET", "/supergraph")
            .expect_at_least(1)
            .with_status(200)
            .with_header("content-type", "text/plain")
            .with_header("etag", "2")
            .with_body(supergraph2_sdl)
            .create();

        wait_until_mock_matched(&mock2)
            .await
            .expect("Expected mock2 to be matched");

        // wait for the router to finish applying the new supergraph state before asserting;
        // the mock being matched only means the poller fetched the sdl, not that the router
        // has finished rebuilding its query planner
        router.wait_for_ready(None).await;

        let res_new_supergraph = router
            .send_graphql_request("{ users { id name reviews { id body } } }", None, None)
            .await;

        let body_json = res_new_supergraph.json_body().await;

        assert!(body_json["data"].is_null());
        assert!(body_json["errors"].is_array());
    }

    #[ntex::test]
    async fn should_be_resilient_to_supergraph_polling_errors() {
        let mut server = mockito::Server::new_async().await;
        let host = server.host_with_port();

        // We want: Initial (200) -> Error (429) -> Error (404) -> Final (200)
        let mock_initial = server
            .mock("GET", "/supergraph")
            .expect_at_least(1)
            .with_status(200)
            .with_header("content-type", "text/plain")
            .with_body("type Query { initial: String }")
            .create();

        let router = TestRouter::builder()
            .inline_config(format!(
                r#"
                supergraph:
                  source: hive
                  endpoint: http://{host}/supergraph
                  key: dummy_key
                  poll_interval: 200ms
                "#,
            ))
            .build()
            .start()
            .await;

        mock_initial.assert();

        // Check if initial supergraph is working
        let res = router
            .send_graphql_request(
                r#"{ __type(name: "Query") { fields { name } } }"#,
                None,
                None,
            )
            .await;

        assert_eq!(res.status(), 200);
        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
        {
          "data": {
            "__type": {
              "fields": [
                {
                  "name": "initial"
                }
              ]
            }
          }
        }
        "#);
        mock_initial.remove();

        let mock_404 = server
            .mock("GET", "/supergraph")
            .expect_at_least(1)
            .with_status(404)
            .create();

        wait_until_mock_matched(&mock_404)
            .await
            .expect("Expected to match 404 mock");

        // Router should still be using the initial supergraph
        let res = router
            .send_graphql_request(
                r#"{ __type(name: "Query") { fields { name } } }"#,
                None,
                None,
            )
            .await;
        assert_eq!(res.status(), 200);
        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
        {
          "data": {
            "__type": {
              "fields": [
                {
                  "name": "initial"
                }
              ]
            }
          }
        }
        "#);

        mock_404.remove();

        let mock_final = server
            .mock("GET", "/supergraph")
            .expect_at_least(1)
            .with_status(200)
            .with_header("content-type", "text/plain")
            .with_body("type Query { updated: String }")
            .create();

        wait_until_mock_matched(&mock_final)
            .await
            .expect("Expected to match final mock");
        mock_final.assert();

        // Check if final supergraph is working
        let res = router
            .send_graphql_request(
                r#"{ __type(name: "Query") { fields { name } } }"#,
                None,
                None,
            )
            .await;
        assert_eq!(res.status(), 200);
        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
        {
          "data": {
            "__type": {
              "fields": [
                {
                  "name": "updated"
                }
              ]
            }
          }
        }
        "#);
    }
}
