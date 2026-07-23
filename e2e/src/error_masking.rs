#[cfg(test)]
mod error_masking_e2e_tests {
    use crate::testkit::{
        ClientResponseExt, EnvVarsGuard, RequestLike, ResponseLike, TestRouter, TestSubgraphs,
    };

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
    async fn should_not_mask_non_subgraph_errors() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let subgraphs_url = subgraphs.url();

        let router = TestRouter::builder()
            .inline_config(format!(
                r#"
                supergraph:
                  source: file
                  path: supergraph.graphql
                error_masking:
                  all:
                    enabled: true
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
            .send_graphql_request("{ i bad query", None, None)
            .await;

        insta::assert_snapshot!(
          res.json_body_string_pretty().await,
          @r#"
        {
          "errors": [
            {
              "message": "Failed to parse GraphQL operation: Parse error at 1:14\nUnexpected end of input\nExpected }\n",
              "extensions": {
                "code": "GRAPHQL_PARSE_FAILED"
              }
            }
          ]
        }
        "#
        );
    }

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
                "code": "SUBREQUEST_HTTP_ERROR",
                "service": "products",
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
                "service": "products",
                "affectedPath": "me.reviews.@.product"
              }
            },
            {
              "message": "Unexpected error",
              "extensions": {
                "code": "SUBREQUEST_HTTP_ERROR",
                "service": "products",
                "affectedPath": "me.reviews.@.product",
                "http": {
                  "status": 500
                }
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
                  enabled: false
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
                "service": "products",
                "affectedPath": "me.reviews.@.product"
              }
            },
            {
              "message": "Subgraph 'products' responded with an invalid HTTP status code '500 Internal Server Error'",
              "extensions": {
                "code": "SUBREQUEST_HTTP_ERROR",
                "service": "products",
                "affectedPath": "me.reviews.@.product",
                "http": {
                  "status": 500
                }
              }
            }
          ]
        }
        "#
        );
    }

    #[ntex::test]
    async fn should_allow_to_disable_all_masking_via_env_var() {
        let subgraphs = TestSubgraphs::builder()
            .with_on_request(failing_products_subgraph)
            .build()
            .start()
            .await;

        let _env_var_guard = EnvVarsGuard::new()
            .set("DISABLE_SUBGRAPH_ERROR_MASKING", "true")
            .apply()
            .await;

        let router = TestRouter::builder()
            .inline_config(config_boilerplate(
                &subgraphs.url(),
                r#"error_masking:
                all:
                  enabled: true # enabled here, but disabled via env var
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
                "service": "products",
                "affectedPath": "me.reviews.@.product"
              }
            },
            {
              "message": "Subgraph 'products' responded with an invalid HTTP status code '500 Internal Server Error'",
              "extensions": {
                "code": "SUBREQUEST_HTTP_ERROR",
                "service": "products",
                "affectedPath": "me.reviews.@.product",
                "http": {
                  "status": 500
                }
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
                "service": "products",
                "affectedPath": "me.reviews.@.product"
              }
            },
            {
              "message": "shit happens",
              "extensions": {
                "code": "SUBREQUEST_HTTP_ERROR",
                "service": "products",
                "affectedPath": "me.reviews.@.product",
                "http": {
                  "status": 500
                }
              }
            }
          ]
        }
        "#
        );
    }

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
                  enabled: true
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
            },
            {
              "message": "Unexpected error",
              "extensions": {
                "code": "SUBREQUEST_HTTP_ERROR"
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
                  enabled: true
                  extensions:
                    mode: deny
                    keys:
                      - code
                      - service
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

    #[ntex::test]
    async fn per_subgraph_extensions_config_override() {
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
                  enabled: true
                  extensions:
                    mode: deny
                    keys:
                      - code
                      - service
                subgraphs:
                  products:
                    extensions:
                      mode: deny
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
                "service": "products",
                "affectedPath": "me.reviews.@.product"
              }
            }
          ]
        }
        "#
        );
    }

    #[ntex::test]
    async fn per_subgraph_message_config_override() {
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
                  enabled: true
                subgraphs:
                  products:
                    enabled: false
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
              "message": "Failed to send request to subgraph: client error (Connect)",
              "extensions": {
                "code": "SUBREQUEST_HTTP_ERROR",
                "service": "products",
                "affectedPath": "me.reviews.@.product"
              }
            }
          ]
        }
        "#
        );
    }

    #[ntex::test]
    async fn per_subgraph_without_extensions_inherits_all_extensions() {
        // A subgraph listed under `subgraphs` that sets only `error_message` still inherits
        // `all.extensions`; only the fields it explicitly sets override `all`.
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
                  enabled: true
                  extensions:
                    mode: allow
                    keys:
                      - code
                subgraphs:
                  products:
                    enabled: false
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
                "code": "DOWNSTREAM_SERVICE_ERROR"
              }
            },
            {
              "message": "Subgraph 'products' responded with an invalid HTTP status code '500 Internal Server Error'",
              "extensions": {
                "code": "SUBREQUEST_HTTP_ERROR"
              }
            }
          ]
        }
        "#
        );
    }

    #[ntex::test]
    async fn full_config_override_per_subgraph() {
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
                  enabled: true
                  extensions:
                    mode: allow
                    keys: [] # allow none!
                subgraphs:
                  products:
                    enabled: false
                    extensions:
                      mode: deny
                      keys: [] # dont deny anything!
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
              "message": "Failed to send request to subgraph: client error (Connect)",
              "extensions": {
                "code": "SUBREQUEST_HTTP_ERROR",
                "service": "products",
                "affectedPath": "me.reviews.@.product"
              }
            }
          ]
        }
        "#
        );
    }
}
