#[cfg(test)]
mod override_subgraph_urls_e2e_tests {
    use ntex::web::test;
    use sonic_rs::{from_slice, json, Value};

    use crate::testkit::{
        init_graphql_request, init_router_from_config_file, wait_for_readiness, SubgraphsServer,
    };

    #[ntex::test]
    async fn should_not_deadlock_when_overriding_subgraph_timeout_statically() {
        let _subgraphs_server = SubgraphsServer::start().await;
        let app = init_router_from_config_file("configs/timeout_per_subgraph_static.router.yaml")
            .await
            .unwrap();
        wait_for_readiness(&app.app).await;

        let req1 = init_graphql_request("{ users { id } }", None);
        let req2 = init_graphql_request("{ users { id } }", None);
        let req3 = init_graphql_request("{ users { id } }", None);
        let req4 = init_graphql_request("{ users { id } }", None);

        let (resp1, resp2, resp3, resp4) = tokio::join!(
            test::call_service(&app.app, req1.to_request()),
            test::call_service(&app.app, req2.to_request()),
            test::call_service(&app.app, req3.to_request()),
            test::call_service(&app.app, req4.to_request())
        );

        assert!(resp1.status().is_success(), "Expected 200 OK");
        assert!(resp2.status().is_success(), "Expected 200 OK");
        assert!(resp3.status().is_success(), "Expected 200 OK");
        assert!(resp4.status().is_success(), "Expected 200 OK");

        let expected_json = json!({
          "data": {
            "users": [
              {
                "id": "1"
              },
              {
                "id": "2"
              },
              {
                "id": "3"
              },
              {
                "id": "4"
              },
              {
                "id": "5"
              },
              {
                "id": "6"
              }
            ]
          }
        });

        for resp in [resp1, resp2, resp3, resp4] {
            let body = test::read_body(resp).await;
            let json_body: Value = from_slice(&body).unwrap();
            assert_eq!(json_body, expected_json);
        }
    }

    #[ntex::test]
    async fn should_not_deadlock_when_overriding_subgraph_timeout_dynamically() {
        let _subgraphs_server = SubgraphsServer::start().await;
        let app = init_router_from_config_file("configs/timeout_per_subgraph_dynamic.router.yaml")
            .await
            .unwrap();
        wait_for_readiness(&app.app).await;

        // We want to ensure that concurrent requests with different timeout settings
        // do not cause deadlocks. We are not testing the actual timeout duration here.
        let req1 = init_graphql_request("{ users { id } }", None).header("x-timeout", "short");
        let req2 = init_graphql_request("{ users { id } }", None).header("x-timeout", "long");
        let req3 = init_graphql_request("{ users { id } }", None).header("x-timeout", "short");
        let req4 = init_graphql_request("{ users { id } }", None).header("x-timeout", "long");

        let (resp1, resp2, resp3, resp4) = tokio::join!(
            test::call_service(&app.app, req1.to_request()),
            test::call_service(&app.app, req2.to_request()),
            test::call_service(&app.app, req3.to_request()),
            test::call_service(&app.app, req4.to_request())
        );

        assert!(resp1.status().is_success(), "Expected 200 OK");
        assert!(resp2.status().is_success(), "Expected 200 OK");
        assert!(resp3.status().is_success(), "Expected 200 OK");
        assert!(resp4.status().is_success(), "Expected 200 OK");

        let expected_json = json!({
          "data": {
            "users": [
              {
                "id": "1"
              },
              {
                "id": "2"
              },
              {
                "id": "3"
              },
              {
                "id": "4"
              },
              {
                "id": "5"
              },
              {
                "id": "6"
              }
            ]
          }
        });

        for resp in [resp1, resp2, resp3, resp4] {
            let body = test::read_body(resp).await;
            let json_body: Value = from_slice(&body).unwrap();
            assert_eq!(json_body, expected_json);
        }
    }
}
