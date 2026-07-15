#[cfg(test)]
mod error_handling_e2e_tests {
    use crate::testkit::{ClientResponseExt, ResponseLike, TestRouter, TestSubgraphs};

    #[ntex::test]
    async fn should_continue_execution_when_a_subgraph_is_down() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let subgraphs_url = subgraphs.url();

        let router = TestRouter::builder()
            // we dont set subgraphs avoiding the port change in the supergrph.
            // we want this because we are here testing the overrides
            // .with_subgraphs(&subgraphs)
            .inline_config(format!(
                r#"
                  supergraph:
                    source: file
                    path: supergraph.graphql
                  error_masking:
                    all:
                      error_message: false
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
                "code": "SUBREQUEST_HTTP_ERROR",
                "service": "products",
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

    // Subgraph returns non-200, with a valid GraphQL body, and also a custom subgraph error in `errors`
    #[ntex::test]
    async fn should_report_error_when_subgraph_responds_with_non_200_and_custom_error() {
        let subgraphs = TestSubgraphs::builder()
            .with_on_request(|req| {
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
                    Some(
                        r#"{"errors":[{"message":"Internal server error from products"}]}"#
                            .to_string(),
                    ),
                    Some(headers),
                ))
            })
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
                  error_masking:
                    all:
                      error_message: false
                  override_subgraph_urls:
                    subgraphs:
                      accounts:
                        url: "{subgraphs_url}/accounts"
                      reviews:
                        url: "{subgraphs_url}/reviews"
                      products:
                        url: "{subgraphs_url}/products"
                "#,
            ))
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request("{ me { reviews { id product { upc name } } } }", None, None)
            .await;

        assert!(res.status().is_success(), "Expected 200 OK");

        // The subgraph's own error is propagated AND a `SUBREQUEST_HTTP_ERROR`
        // carrying the HTTP status is injected, while partial data is kept.
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

    // Subgraph returns non-200, with a valid GraphQL body, no custom errors
    #[ntex::test]
    async fn should_report_error_when_subgraph_responds_with_non_200_no_custom_error() {
        let subgraphs = TestSubgraphs::builder()
            .with_on_request(|req| {
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
                    Some(r#"{"data": null}"#.to_string()),
                    Some(headers),
                ))
            })
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
                  error_masking:
                    all:
                      error_message: false
                  override_subgraph_urls:
                    subgraphs:
                      accounts:
                        url: "{subgraphs_url}/accounts"
                      reviews:
                        url: "{subgraphs_url}/reviews"
                      products:
                        url: "{subgraphs_url}/products"
                "#,
            ))
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request("{ me { reviews { id product { upc name } } } }", None, None)
            .await;

        assert!(res.status().is_success(), "Expected 200 OK");

        // The subgraph's own error is propagated AND a `SUBREQUEST_HTTP_ERROR`
        // carrying the HTTP status is injected, while partial data is kept.
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
              "message": "Subgraph returned malformed response",
              "extensions": {
                "code": "SUBREQUEST_MALFORMED_RESPONSE",
                "service": "products",
                "affectedPath": "me.reviews.@.product"
              }
            }
          ]
        }
        "#
        );
    }

    // Subgraph returns non-200, with an invalid GraphQL body
    #[ntex::test]
    async fn should_report_error_when_subgraph_responds_with_non_200_invalid_graphql_body() {
        let subgraphs = TestSubgraphs::builder()
            .with_on_request(|req| {
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
                    Some(r#"{"error": "oopsi boopsi"}"#.to_string()),
                    Some(headers),
                ))
            })
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
                  error_masking:
                    all:
                      error_message: false
                  override_subgraph_urls:
                    subgraphs:
                      accounts:
                        url: "{subgraphs_url}/accounts"
                      reviews:
                        url: "{subgraphs_url}/reviews"
                      products:
                        url: "{subgraphs_url}/products"
                "#,
            ))
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request("{ me { reviews { id product { upc name } } } }", None, None)
            .await;

        assert!(res.status().is_success(), "Expected 200 OK");

        // The subgraph's own error is propagated AND a `SUBREQUEST_HTTP_ERROR`
        // carrying the HTTP status is injected, while partial data is kept.
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
              "message": "Subgraph returned malformed response",
              "extensions": {
                "code": "SUBREQUEST_MALFORMED_RESPONSE",
                "service": "products",
                "affectedPath": "me.reviews.@.product"
              }
            }
          ]
        }
        "#
        );
    }

    // Subgraph returns 200, with a invalid GraphQL body
    #[ntex::test]
    async fn should_report_error_when_subgraph_responds_with_200_invalid_graphql_body() {
        let subgraphs = TestSubgraphs::builder()
            .with_on_request(|req| {
                if req.path != "/products" {
                    return None;
                }
                let mut headers = http::HeaderMap::new();
                headers.insert(
                    http::header::CONTENT_TYPE,
                    http::HeaderValue::from_static("application/json"),
                );
                Some(ResponseLike::new(
                    axum::http::StatusCode::OK,
                    Some(r#"{"error": "oopsi boopsi"}"#.to_string()),
                    Some(headers),
                ))
            })
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
                  error_masking:
                    all:
                      error_message: false
                  override_subgraph_urls:
                    subgraphs:
                      accounts:
                        url: "{subgraphs_url}/accounts"
                      reviews:
                        url: "{subgraphs_url}/reviews"
                      products:
                        url: "{subgraphs_url}/products"
                "#,
            ))
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request("{ me { reviews { id product { upc name } } } }", None, None)
            .await;

        assert!(res.status().is_success(), "Expected 200 OK");

        // The subgraph's own error is propagated AND a `SUBREQUEST_HTTP_ERROR`
        // carrying the HTTP status is injected, while partial data is kept.
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
              "message": "Subgraph returned malformed response",
              "extensions": {
                "code": "SUBREQUEST_MALFORMED_RESPONSE",
                "service": "products",
                "affectedPath": "me.reviews.@.product"
              }
            }
          ]
        }
        "#
        );
    }

    // Subgraph returns 200, with a GraphQL body, with invalid content type
    #[ntex::test]
    async fn should_report_error_when_subgraph_responds_with_200_invalid_content_type() {
        let subgraphs = TestSubgraphs::builder()
            .with_on_request(|req| {
                if req.path != "/products" {
                    return None;
                }
                let mut headers = http::HeaderMap::new();
                headers.insert(
                    http::header::CONTENT_TYPE,
                    http::HeaderValue::from_static("text/html"),
                );
                Some(ResponseLike::new(
                    axum::http::StatusCode::OK,
                    Some(r#"{"errors": [{"message": "test"}]}"#.to_string()),
                    Some(headers),
                ))
            })
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
                  error_masking:
                    all:
                      error_message: false
                  override_subgraph_urls:
                    subgraphs:
                      accounts:
                        url: "{subgraphs_url}/accounts"
                      reviews:
                        url: "{subgraphs_url}/reviews"
                      products:
                        url: "{subgraphs_url}/products"
                "#,
            ))
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request("{ me { reviews { id product { upc name } } } }", None, None)
            .await;

        assert!(res.status().is_success(), "Expected 200 OK");

        // The subgraph's own error is propagated AND a `SUBREQUEST_HTTP_ERROR`
        // carrying the HTTP status is injected, while partial data is kept.
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
              "message": "Invalid content type returned from subgraph 'products': 'text/html'",
              "extensions": {
                "code": "SUBREQUEST_MALFORMED_RESPONSE",
                "service": "products",
                "affectedPath": "me.reviews.@.product"
              }
            }
          ]
        }
        "#
        );
    }

    // Subgraph returns 200, with invalid JSON body
    #[ntex::test]
    async fn should_report_error_when_subgraph_responds_with_200_invalid_json_body() {
        let subgraphs = TestSubgraphs::builder()
            .with_on_request(|req| {
                if req.path != "/products" {
                    return None;
                }
                let mut headers = http::HeaderMap::new();
                headers.insert(
                    http::header::CONTENT_TYPE,
                    http::HeaderValue::from_static("application/json"),
                );
                Some(ResponseLike::new(
                    axum::http::StatusCode::OK,
                    Some(r#"bad json"#.to_string()),
                    Some(headers),
                ))
            })
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
                  error_masking:
                    all:
                      error_message: false
                  override_subgraph_urls:
                    subgraphs:
                      accounts:
                        url: "{subgraphs_url}/accounts"
                      reviews:
                        url: "{subgraphs_url}/reviews"
                      products:
                        url: "{subgraphs_url}/products"
                "#,
            ))
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request("{ me { reviews { id product { upc name } } } }", None, None)
            .await;

        assert!(res.status().is_success(), "Expected 200 OK");

        // The subgraph's own error is propagated AND a `SUBREQUEST_HTTP_ERROR`
        // carrying the HTTP status is injected, while partial data is kept.
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
              "message": "Failed to deserialize subgraph response: Invalid JSON value at line 1 column 1\n\n\tbad json\n\t^.......\n",
              "extensions": {
                "code": "SUBREQUEST_MALFORMED_RESPONSE",
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
