#[cfg(test)]
mod jwt_e2e_tests {
    use std::{sync::Arc, time::Duration};

    use ntex::{time, web::test};
    use sonic_rs::{from_slice, JsonValueTrait, Value};

    use crate::testkit::{
        init_graphql_request, init_router_from_config_inline, wait_for_readiness, SubgraphsServer,
    };

    #[ntex::test]
    async fn should_clear_internal_caches_when_supergraph_changes() {
        let mut server = mockito::Server::new_async().await;
        let host = server.host_with_port();
        let mock1 = server
            .mock("GET", "/supergraph")
            .expect(1)
            .with_status(200)
            .with_header("content-type", "text/plain")
            .with_body("type Query { dummy: String }")
            .create();

        let mock2 = server
            .mock("GET", "/supergraph")
            .expect_at_least(1)
            .with_status(200)
            .with_header("content-type", "text/plain")
            .with_body("type Query { dummyNew: NewType } type NewType { id: ID! }")
            .create();

        let test_app = init_router_from_config_inline(&format!(
            r#"supergraph:
              source: hive
              endpoint: http://{host}/supergraph
              key: dummy_key
              poll_interval: 500ms
        "#,
        ))
        .await
        .expect("failed to start router");

        wait_for_readiness(&test_app.app).await;
        mock1.assert();

        assert_eq!(test_app.schema_state.validate_cache.entry_count(), 0);
        assert_eq!(test_app.schema_state.plan_cache.entry_count(), 0);
        assert_eq!(test_app.schema_state.normalize_cache.entry_count(), 0);

        let resp = test::call_service(
            &test_app.app,
            init_graphql_request("{ __schema { types { name } } }", None).to_request(),
        )
        .await;

        assert!(resp.status().is_success(), "Expected 200 OK");

        // Flush the caches
        test_app.flush_internal_cache().await;

        // Now it should have the record
        assert_eq!(test_app.schema_state.validate_cache.entry_count(), 1);
        assert_eq!(test_app.schema_state.plan_cache.entry_count(), 1);
        assert_eq!(test_app.schema_state.normalize_cache.entry_count(), 1);

        // Now let's wait a bit and let the service re-load and get the new supergraph
        time::sleep(Duration::from_millis(600)).await;
        mock2.assert();
        test_app.flush_internal_cache().await;

        // Now cache should be empty again, if supergraph has changes
        assert_eq!(test_app.schema_state.validate_cache.entry_count(), 0);
        assert_eq!(test_app.schema_state.plan_cache.entry_count(), 0);
        assert_eq!(test_app.schema_state.normalize_cache.entry_count(), 0);
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
        std::env::set_var("SUBGRAPH_DELAY_MS", "500");
        let _subgraphs_server = SubgraphsServer::start().await;

        let mut server = mockito::Server::new_async().await;
        let host = server.host_with_port();
        let supergraph1_sdl = include_str!("../supergraph.graphql");

        // First supergraph
        let mock1 = server
            .mock("GET", "/supergraph")
            .expect(1)
            .with_status(200)
            .with_header("content-type", "text/plain")
            .with_header("etag", "1")
            .with_body(supergraph1_sdl)
            .create();

        // Second supergraph
        let mock2 = server
            .mock("GET", "/supergraph")
            .expect_at_least(1)
            .with_status(200)
            .with_header("content-type", "text/plain")
            .with_header("etag", "2")
            .with_body(
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
            )
            .create();

        let test_app = Arc::new(
            init_router_from_config_inline(&format!(
                r#"supergraph:
              source: hive
              endpoint: http://{host}/supergraph
              key: dummy_key
              poll_interval: 300ms
        "#,
            ))
            .await
            .expect("failed to start router"),
        );

        wait_for_readiness(&test_app.app).await;
        mock1.assert();

        let app_clone = test_app.app.clone();
        let resp_handle = ntex::rt::spawn(async move {
            let response = test::call_service(
                &app_clone.clone(),
                init_graphql_request("{ users { id name reviews { id body } } }", None)
                    .to_request(),
            )
            .await;
            assert!(response.status().is_success(), "Expected 200 OK");
            let body = test::read_body(response).await;
            let json_body: Value = from_slice(&body).unwrap();

            json_body
        });

        ntex::time::sleep(Duration::from_millis(100)).await;
        let resp = resp_handle.await.unwrap();
        mock2.assert();

        assert!(resp["data"].is_object());
        assert!(resp["errors"].is_null());

        let response_new_supergraph = test::call_service(
            &test_app.app,
            init_graphql_request("{ users { id name reviews { id body } } }", None).to_request(),
        )
        .await;

        let body = test::read_body(response_new_supergraph).await;
        let json_body: Value = from_slice(&body).unwrap();

        assert!(json_body["data"].is_null());
        assert!(json_body["errors"].is_array());
    }
}
