#[cfg(test)]
mod error_masking_e2e_tests {
    use crate::testkit::{ClientResponseExt, RequestLike, ResponseLike, TestRouter, TestSubgraphs};

    fn failing_products_subgraph(req: RequestLike) -> Option<ResponseLike> {
        if req.path != "/products" {
            return None;
        }
        let mut headers = http::HeaderMap::new();
        headers.insert(
            http::header::CONTENT_TYPE,
            http::HeaderValue::from_static("application/json"),
        );
        Some(ResponseLike::new(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Some(r#"{"errors":[{"message":"Internal server error from products"}]}"#.to_string()),
            Some(headers),
        ))
    }

    fn config_boilerplate(subgraphs_url: &str, error_masking: &str) -> String {
        format!(
            r#"
          supergraph:
            source: file
            path: supergraph.graphql
          override_subgraph_urls:
            subgraphs:
              accounts:
                url: "{subgraphs_url}/accounts"
              reviews:
                url: "{subgraphs_url}/reviews"
              products:
                url: "{subgraphs_url}/products"
          {}
          "#,
            error_masking
        )
    }

    static QUERY: &str = "{ me { reviews { id product { upc name } } } }";

    #[ntex::test]
    async fn should_mask_network_error_by_default() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let subgraphs_url = subgraphs.url();

        let router = TestRouter::builder()
            .inline_config(format!(
                r#"
                supergraph:
                  source: file
                  path: supergraph.graphql
                override_subgraph_urls:
                  subgraphs:
                    accounts:
                      url: "{subgraphs_url}/accounts"
                    reviews:
                      url: "{subgraphs_url}/reviews"
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

        insta::assert_snapshot!(
          res.json_body_string_pretty().await,
          @r#"
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
              "message": "Unexpected error",
              "extensions": {
                "code": "SUBGRAPH_REQUEST_FAILURE",
                "serviceName": "products",
                "affectedPath": "me.reviews.@.product"
              }
            }
          ]
        }
        "#
        );
    }

    #[ntex::test]
    async fn should_mask_downstream_service_error_by_default() {
        let subgraphs = TestSubgraphs::builder()
            .with_on_request(failing_products_subgraph)
            .build()
            .start()
            .await;

        let router = TestRouter::builder()
            .inline_config(config_boilerplate(&subgraphs.url(), ""))
            .build()
            .start()
            .await;

        let res = router.send_graphql_request(QUERY, None, None).await;

        assert!(res.status().is_success(), "Expected 200 OK");

        insta::assert_snapshot!(
          res.json_body_string_pretty().await,
          @r#"
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
              "message": "Unexpected error",
              "extensions": {
                "code": "DOWNSTREAM_SERVICE_ERROR",
                "serviceName": "products",
                "affectedPath": "me.reviews.@.product"
              }
            }
          ]
        }
        "#
        );
    }

    #[ntex::test]
    async fn should_allow_to_disable_all_masking() {
        let subgraphs = TestSubgraphs::builder()
            .with_on_request(failing_products_subgraph)
            .build()
            .start()
            .await;

        let router = TestRouter::builder()
            .inline_config(config_boilerplate(
                &subgraphs.url(),
                r#"error_masking:
                all:
                  error_message: false
            "#,
            ))
            .build()
            .start()
            .await;

        let res = router.send_graphql_request(QUERY, None, None).await;

        assert!(res.status().is_success(), "Expected 200 OK");

        insta::assert_snapshot!(
          res.json_body_string_pretty().await,
          @r#"
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
              "message": "Internal server error from products",
              "extensions": {
                "code": "DOWNSTREAM_SERVICE_ERROR",
                "serviceName": "products",
                "affectedPath": "me.reviews.@.product"
              }
            }
          ]
        }
        "#
        );
    }

    #[ntex::test]
    async fn should_allow_to_customize_masking_message() {
        let subgraphs = TestSubgraphs::builder()
            .with_on_request(failing_products_subgraph)
            .build()
            .start()
            .await;

        let router = TestRouter::builder()
            .inline_config(config_boilerplate(
                &subgraphs.url(),
                r#"error_masking:
                redacted_error_message: "shit happens"
            "#,
            ))
            .build()
            .start()
            .await;

        let res = router.send_graphql_request(QUERY, None, None).await;

        assert!(res.status().is_success(), "Expected 200 OK");

        insta::assert_snapshot!(
          res.json_body_string_pretty().await,
          @r#"
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
              "message": "shit happens",
              "extensions": {
                "code": "DOWNSTREAM_SERVICE_ERROR",
                "serviceName": "products",
                "affectedPath": "me.reviews.@.product"
              }
            }
          ]
        }
        "#
        );
    }

    #[ntex::test]
    async fn override_for_one_subgraph() {}

    #[ntex::test]
    async fn extensions_allow_list() {
        let subgraphs = TestSubgraphs::builder()
            .with_on_request(failing_products_subgraph)
            .build()
            .start()
            .await;

        let router = TestRouter::builder()
            .inline_config(config_boilerplate(
                &subgraphs.url(),
                r#"error_masking:
                all:
                  error_message: true
                  extensions:
                    mode: allow
                    keys:
                      - code
            "#,
            ))
            .build()
            .start()
            .await;

        let res = router.send_graphql_request(QUERY, None, None).await;

        assert!(res.status().is_success(), "Expected 200 OK");

        insta::assert_snapshot!(
          res.json_body_string_pretty().await,
          @r#"
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
              "message": "Unexpected error",
              "extensions": {
                "code": "DOWNSTREAM_SERVICE_ERROR"
              }
            }
          ]
        }
        "#
        );
    }

    #[ntex::test]
    async fn extensions_deny_list() {
        let subgraphs = TestSubgraphs::builder()
            .with_on_request(failing_products_subgraph)
            .build()
            .start()
            .await;

        let subgraphs_url = subgraphs.url();
        let router = TestRouter::builder()
            .inline_config(format!(
                r#"
              supergraph:
                source: file
                path: supergraph.graphql
              override_subgraph_urls:
                subgraphs:
                  accounts:
                    url: "{subgraphs_url}/accounts"
                  reviews:
                    url: "{subgraphs_url}/reviews"
                  products:
                    url: "http://0.0.0.0:1000/products"
              error_masking:
                all:
                  error_message: true
                  extensions:
                    mode: deny
                    keys:
                      - code
                      - serviceName
            "#,
            ))
            .build()
            .start()
            .await;

        let res = router.send_graphql_request(QUERY, None, None).await;

        assert!(res.status().is_success(), "Expected 200 OK");

        insta::assert_snapshot!(
          res.json_body_string_pretty().await,
          @r#"
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
              "message": "Unexpected error",
              "extensions": {
                "affectedPath": "me.reviews.@.product"
              }
            }
          ]
        }
        "#
        );
    }
}
