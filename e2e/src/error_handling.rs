#[cfg(test)]
mod error_handling_e2e_tests {
    use ntex::web::test;
    use sonic_rs::{from_slice, to_string_pretty, Value};

    use crate::testkit::{
        init_graphql_request, init_router_from_config_file, wait_for_readiness, SubgraphsServer,
    };

    #[ntex::test]
    async fn should_continue_execution_when_a_subgraph_is_down() {
        let subgraphs_server = SubgraphsServer::start_with_port(4100).await;

        // In the config file, we point the `products` subgraph to a non-existing server
        // to simulate a subgraph being down.
        let app = init_router_from_config_file("configs/error_handling.router.yaml")
            .await
            .unwrap();
        wait_for_readiness(&app.app).await;

        let req = init_graphql_request("{ me { reviews { id product { upc name } } } }", None);
        let resp = test::call_service(&app.app, req.to_request()).await;

        assert!(resp.status().is_success(), "Expected 200 OK");
        let resp_body_bytes = test::read_body(resp).await;
        let resp_json: Value = from_slice(&resp_body_bytes).expect("expected valid JSON response");

        // Router cannot fetch `name` field from `products` subgraph because it's down,
        // but it should still return the rest of the data from `accounts` subgraph.
        insta::assert_snapshot!(
            to_string_pretty(&resp_json).unwrap(),
            @r###"
        {
          "data": {
            "me": {
              "reviews": [
                {
                  "id": "1",
                  "product": {
                    "upc": "1",
                    "name": null
                  }
                },
                {
                  "id": "2",
                  "product": {
                    "upc": "1",
                    "name": null
                  }
                }
              ]
            }
          },
          "errors": [
            {
              "message": "Failed to send request to subgraph \"http://0.0.0.0:9876/products\": client error (Connect)",
              "extensions": {
                "code": "SUBGRAPH_REQUEST_FAILURE",
                "serviceName": "products"
              }
            }
          ]
        }
        "###
        );

        assert_eq!(
            subgraphs_server
                .get_subgraph_requests_log("accounts")
                .await
                .expect("expected requests sent to accounts subgraph")
                .len(),
            1,
            "expected 1 request to accounts subgraph"
        );
    }
}
