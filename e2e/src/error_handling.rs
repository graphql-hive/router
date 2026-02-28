#[cfg(test)]
mod error_handling_e2e_tests {
    use crate::testkit::{ClientResponseExt, TestRouterBuilder, TestSubgraphsBuilder};

    #[ntex::test]
    async fn should_continue_execution_when_a_subgraph_is_down() {
        let subgraphs = TestSubgraphsBuilder::new().build().start().await;
        let subgraphs_addr = subgraphs.addr();

        let router = TestRouterBuilder::new()
            // we dont set subgraphs avoiding the port change in the supergrph.
            // we want this because we are here testing the overrides
            // .with_subgraphs(&subgraphs)
            .inline_config(format!(
                r#"
                  supergraph:
                    source: file
                    path: supergraph.graphql
                  override_subgraph_urls:
                    accounts:
                      url: "http://{subgraphs_addr}/accounts"
                    reviews:
                      url: "http://{subgraphs_addr}/reviews"
                    products:
                      url: "http://0.0.0.0:1000/products"
                "#,
            ))
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request("{ me { reviews { id product { upc name } } } }", None, None)
            .await;

        assert!(res.status().is_success(), "Expected 200 OK");

        // Router cannot fetch `name` field from `products` subgraph because it's down,
        // but it should still return the rest of the data from `accounts` subgraph.
        insta::assert_snapshot!(
            res.json_body_string_pretty().await,
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
              "message": "Failed to send request to subgraph: client error (Connect)",
              "extensions": {
                "code": "SUBGRAPH_REQUEST_FAILURE",
                "serviceName": "products",
                "affectedPath": "me.reviews.@.product"
              }
            }
          ]
        }
        "###
        );

        assert_eq!(
            subgraphs
                .get_requests_log("accounts")
                .expect("expected requests sent to accounts subgraph")
                .len(),
            1,
            "expected 1 request to accounts subgraph"
        );
    }
}
