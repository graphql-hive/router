#[cfg(test)]
mod issues_e2e_tests {
    use crate::testkit::{ClientResponseExt, Started, TestRouter, TestSubgraphs};

    #[ntex::test]
    /// https://github.com/graphql-hive/federation-gateway-audit `src/test-suites/null-keys`
    async fn federation_audit_null_keys() {
        let mut server = mockito::Server::new_async().await;
        let host = server.host_with_port();

        let router = TestRouter::builder()
            .inline_config(format!(
                r#"
                  supergraph:
                    source: file
                    path: src/issues/supergraph.null-keys.graphql
                  override_subgraph_urls:
                    subgraphs:
                      a:
                        url: "http://{host}/a"
                      b:
                        url: "http://{host}/b"
                      c:
                        url: "http://{host}/c"
                  "#
            ))
            .build()
            .start()
            .await;

        let _a = server
            .mock("POST", "/a")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"data":{"bookContainers":[
                    {"book":{"__typename":"Book","upc":"b1"}},
                    {"book":{"__typename":"Book","upc":"b2"}},
                    {"book":{"__typename":"Book","upc":"b3"}}
                ]}}"#,
            )
            .create();

        let _b = server
            .mock("POST", "/b")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"data":{"_entities":[
                    {"__typename":"Book","id":"1"},
                    {"__typename":"Book","id":"2"},
                    null
                ]}}"#,
            )
            .create();

        let _c_invalid = server
            .mock("POST", "/c")
            .match_request(|r| {
                let body = String::from_utf8(r.body().unwrap().clone()).unwrap();
                body.contains(r#"{"__typename":"Book"}"#)
            })
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"data":{"_entities":[
                    {"__typename":"Book","author":{"__typename":"Author","name":"Alice"}},
                    {"__typename":"Book","author":{"__typename":"Author","name":"Bob"}},
                    null
                ]},"errors":[{"message":"Invalid reference","path":["_entities",2]}]}"#,
            )
            .create();
        let _c_ok = server
            .mock("POST", "/c")
            .match_request(|r| {
                let body = String::from_utf8(r.body().unwrap().clone()).unwrap();
                !body.contains(r#"{"__typename":"Book"}"#)
            })
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"data":{"_entities":[
                    {"__typename":"Book","author":{"__typename":"Author","name":"Alice"}},
                    {"__typename":"Book","author":{"__typename":"Author","name":"Bob"}}
                ]}}"#,
            )
            .create();

        let res = router
            .send_graphql_request(
                r#"query { bookContainers { book { upc author { name } } } }"#,
                None,
                None,
            )
            .await;

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
        {
          "data": {
            "bookContainers": [
              {
                "book": {
                  "upc": "b1",
                  "author": {
                    "name": "Alice"
                  }
                }
              },
              {
                "book": {
                  "upc": "b2",
                  "author": {
                    "name": "Bob"
                  }
                }
              },
              {
                "book": {
                  "upc": "b3",
                  "author": null
                }
              }
            ]
          }
        }
        "#);
    }

    #[ntex::test]
    /// https://github.com/graphql-hive/router/issues/880
    async fn issue_880_null_in_required_field() {
        let mut server = mockito::Server::new_async().await;
        let host = server.host_with_port();

        let router = TestRouter::builder()
            .inline_config(format!(
                r#"
                  supergraph:
                    source: file
                    path: src/issues/supergraph.880.graphql
                  query_planner:
                    allow_expose: true
                  override_subgraph_urls:
                    subgraphs:
                      accounts:
                        url: "http://{host}/accounts"
                      products:
                        url: "http://{host}/products"
                  "#
            ))
            .build()
            .start()
            .await;

        // QueryPlan {
        //   Sequence {
        //     Fetch(service: "products") {
        //       {
        //         ad(id: "1") {
        //           id
        //           branch {
        //             __typename
        //             id
        //           }
        //         }
        //       }
        //     },
        let products_query_mock = server
            .mock("POST", "/products")
            .match_request(|r| {
                let body = r.body().unwrap();
                let body_str = String::from_utf8(body.clone()).unwrap();

                body_str.contains("ad(")
            })
            .expect(1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"
                {
                  "data": {
                    "ad": { "id": "1", "branch": { "__typename": "Branch", "id": "branch-1" } }
                  }
                }
                "#,
            )
            .create();

        // Flatten(path: "ad.branch") {
        //   Fetch(service: "accounts") {
        //     {
        //       ... on Branch {
        //         __typename
        //         id
        //       }
        //     } =>
        //     {
        //       ... on Branch {
        //         contactOptions {
        //           email
        //           user {
        //             name
        //             id
        //           }
        //         }
        //       }
        //     }
        //   },
        // },
        let accounts_mock = server
            .mock("POST", "/accounts")
            .expect(1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                "data": {
                  "_entities": [
                    { "__typename": "Branch", "id": "branch-1", "contactOptions": null }
                  ]
                }
              }
              "#,
            )
            .create();

        //     Flatten(path: "ad") {
        //       Fetch(service: "products") {
        //         {
        //           ... on Ad {
        //             __typename
        //             branch {
        //               contactOptions {
        //                 email
        //                 user {
        //                   id
        //                   name
        //                 }
        //               }
        //             }
        //             id
        //           }
        //         } =>
        //         {
        //           ... on Ad {
        //             contactOptions {
        //               email
        //             }
        //           }
        //         }
        //       },
        //     },
        //   },
        // },
        let _products_entities_mock_valid_json = server
            .mock("POST", "/products")
            .match_request(|r| {
                let body = r.body().unwrap();
                let body_str = String::from_utf8(body.clone()).unwrap();
                if !body_str.contains("$representations") {
                    return false;
                }

                sonic_rs::from_slice::<sonic_rs::Value>(body).is_ok()
            })
            .expect(1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                "data": {
                  "_entities": [
                    { "__typename": "Ad", "contactOptions": null }
                  ]
                }
              }
              "#,
            )
            .create();
        let _products_entities_mock_invalid_json = server
            .mock("POST", "/products")
            .match_request(|r| {
                let body = r.body().unwrap();
                let body_str = String::from_utf8(body.clone()).unwrap();
                if !body_str.contains("$representations") {
                    return false;
                }

                sonic_rs::from_slice::<sonic_rs::Value>(body).is_err()
            })
            .expect(1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                "data": {
                  "_entities": [null]
                },
                "errors": [
                  { "message": "invalid json" }
                ]
              }
              "#,
            )
            .create();

        let res = router
            .send_graphql_request(
                "{ ad(id: \"1\") { id contactOptions { email } } }",
                None,
                None,
            )
            .await;

        accounts_mock.assert();
        products_query_mock.assert();

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
        {
          "data": {
            "ad": {
              "id": "1",
              "contactOptions": null
            }
          }
        }
        "#);
    }

    async fn build_issue_966_router(host: &str) -> TestRouter<Started> {
        TestRouter::builder()
            .inline_config(format!(
                r#"
                  supergraph:
                    source: file
                    path: src/issues/supergraph.966.graphql
                  query_planner:
                    allow_expose: true
                  override_subgraph_urls:
                    subgraphs:
                      labels:
                        url: "http://{host}/labels"
                      products:
                        url: "http://{host}/products"
                  "#
            ))
            .build()
            .start()
            .await
    }

    #[ntex::test]
    /// https://github.com/graphql-hive/router/issues/966
    async fn issue_966_custom_scalar_root_and_abstract_paths() {
        let mut server = mockito::Server::new_async().await;
        let router = build_issue_966_router(&server.host_with_port()).await;

        let labels_mock = server
            .mock("POST", "/labels")
            .match_request(|r| {
                let body = r.body().unwrap();
                let body_str = String::from_utf8(body.clone()).unwrap();

                body_str.contains("labels")
                    && body_str.contains("labelsArray")
                    && body_str.contains("labelsText")
                    && body_str.contains("labelsNumber")
                    && body_str.contains("labelsBool")
                    && body_str.contains("labelsNull")
                    && body_str.contains("abstractThing")
                    && body_str.contains("abstractThings")
                    && body_str.contains("catalog")
            })
            .expect(1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"
                {
                  "data": {
                    "labels": {
                      "generic.learnMore.button\t": "Learn more"
                    },
                    "renamed": {
                      "generic.learnMore.button\t": "Learn more"
                    },
                    "labelsArray": [
                      "one",
                      {
                        "generic.learnMore.button\t": "Learn more"
                      },
                      1,
                      true,
                      null
                    ],
                    "labelsText": "plain text",
                    "labelsNumber": 42,
                    "labelsBool": true,
                    "labelsNull": null,
                    "catalog": {
                      "metadata": {
                        "nested.key\t": "nested value"
                      },
                      "renamedMetadata": {
                        "nested.key\t": "nested value"
                      },
                      "metadataList": [
                        {
                          "list.key\t": "list value"
                        },
                        [
                          "x",
                          {
                            "deep.key\t": "deep value"
                          }
                        ]
                      ]
                    },
                    "abstractThing": {
                      "__typename": "LabeledThing",
                      "metadata": {
                        "abstract.inline\t": "inline value"
                      }
                    },
                    "abstractThings": [
                      {
                        "__typename": "LabeledThing",
                        "metadata": {
                          "abstract.list\t": "first"
                        }
                      },
                      {
                        "__typename": "PlainThing"
                      }
                    ]
                  },
                  "extensions": {
                    "trace": {
                      "raw": {
                        "shouldStayStructured": true
                      }
                    }
                  }
                }
                "#,
            )
            .create();

        let res = router
            .send_graphql_request(
                r#"
                {
                  labels
                  renamed: labels
                  labelsArray
                  labelsText
                  labelsNumber
                  labelsBool
                  labelsNull
                  abstractThing {
                    __typename
                    ...AbstractMetadata
                  }
                  abstractThings {
                    __typename
                    ... on LabeledThing {
                      metadata
                    }
                  }
                  catalog {
                    metadata
                    renamedMetadata: metadata
                    metadataList
                  }
                }

                fragment AbstractMetadata on LabeledThing {
                  metadata
                }
                "#,
                None,
                None,
            )
            .await;

        labels_mock.assert();

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
        {
          "data": {
            "labels": {
              "generic.learnMore.button\t": "Learn more"
            },
            "renamed": {
              "generic.learnMore.button\t": "Learn more"
            },
            "labelsArray": [
              "one",
              {
                "generic.learnMore.button\t": "Learn more"
              },
              1,
              true,
              null
            ],
            "labelsText": "plain text",
            "labelsNumber": 42,
            "labelsBool": true,
            "labelsNull": null,
            "abstractThing": {
              "__typename": "LabeledThing",
              "metadata": {
                "abstract.inline\t": "inline value"
              }
            },
            "abstractThings": [
              {
                "__typename": "LabeledThing",
                "metadata": {
                  "abstract.list\t": "first"
                }
              },
              {
                "__typename": "PlainThing"
              }
            ],
            "catalog": {
              "metadata": {
                "nested.key\t": "nested value"
              },
              "renamedMetadata": {
                "nested.key\t": "nested value"
              },
              "metadataList": [
                {
                  "list.key\t": "list value"
                },
                [
                  "x",
                  {
                    "deep.key\t": "deep value"
                  }
                ]
              ]
            }
          }
        }
        "#);
    }

    #[ntex::test]
    /// https://github.com/graphql-hive/router/issues/966
    async fn issue_966_custom_scalar_conditional_abstract_paths() {
        let mut server = mockito::Server::new_async().await;
        let router = build_issue_966_router(&server.host_with_port()).await;

        let labels_abstract_conditional_mock = server
            .mock("POST", "/labels")
            .match_request(|r| {
                let body = r.body().unwrap();
                let body_str = String::from_utf8(body.clone()).unwrap();

                body_str.contains("abstractThing")
                    && body_str.contains("$include")
                    && body_str.contains("$skip")
            })
            .expect(2)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"
                {
                  "data": {
                    "abstractThing": {
                      "__typename": "LabeledThing",
                      "metadata": {
                        "conditional.field\t": "field value"
                      },
                      "gatedMetadata": {
                        "conditional.fragment\t": "fragment value"
                      }
                    }
                  }
                }
                "#,
            )
            .create();

        let included_res = router
            .send_graphql_request(
                r#"
                query($include: Boolean!, $skip: Boolean!) {
                  abstractThing {
                    __typename
                    ... on LabeledThing {
                      metadata @skip(if: $skip) @include(if: $include)
                    }
                    ... on LabeledThing @skip(if: $skip) @include(if: $include) {
                      gatedMetadata: metadata
                    }
                  }
                }
                "#,
                Some(sonic_rs::json!({
                    "include": true,
                    "skip": false,
                })),
                None,
            )
            .await;

        let skipped_res = router
            .send_graphql_request(
                r#"
                query($include: Boolean!, $skip: Boolean!) {
                  abstractThing {
                    __typename
                    ... on LabeledThing {
                      metadata @skip(if: $skip) @include(if: $include)
                    }
                    ... on LabeledThing @skip(if: $skip) @include(if: $include) {
                      gatedMetadata: metadata
                    }
                  }
                }
                "#,
                Some(sonic_rs::json!({
                    "include": false,
                    "skip": false,
                })),
                None,
            )
            .await;

        labels_abstract_conditional_mock.assert();

        insta::assert_snapshot!(included_res.json_body_string_pretty().await, @r#"
        {
          "data": {
            "abstractThing": {
              "__typename": "LabeledThing",
              "metadata": {
                "conditional.field\t": "field value"
              },
              "gatedMetadata": {
                "conditional.fragment\t": "fragment value"
              }
            }
          }
        }
        "#);

        insta::assert_snapshot!(skipped_res.json_body_string_pretty().await, @r#"
        {
          "data": {
            "abstractThing": {
              "__typename": "LabeledThing"
            }
          }
        }
        "#);
    }

    #[ntex::test]
    /// https://github.com/graphql-hive/router/issues/966
    async fn issue_966_custom_scalar_direct_root_product_field() {
        let mut server = mockito::Server::new_async().await;
        let router = build_issue_966_router(&server.host_with_port()).await;

        let product_query_mock = server
            .mock("POST", "/products")
            .match_request(|r| {
                let body = r.body().unwrap();
                let body_str = String::from_utf8(body.clone()).unwrap();

                body_str.contains("product(id:") && !body_str.contains("_entities")
            })
            .expect(1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"
                {
                  "data": {
                    "product": {
                      "id": "p1",
                      "metadata": {
                        "entity.root\t": "entity root value"
                      }
                    }
                  }
                }
                "#,
            )
            .create();

        let res = router
            .send_graphql_request(
                r#"
                {
                  product(id: "p1") {
                    id
                    metadata
                    renamedMetadata: metadata
                  }
                }
                "#,
                None,
                None,
            )
            .await;

        product_query_mock.assert();

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
        {
          "data": {
            "product": {
              "id": "p1",
              "metadata": {
                "entity.root\t": "entity root value"
              },
              "renamedMetadata": null
            }
          }
        }
        "#);
    }

    #[ntex::test]
    /// https://github.com/graphql-hive/router/issues/966
    async fn issue_966_custom_scalar_single_entity_fetch() {
        let mut server = mockito::Server::new_async().await;
        let router = build_issue_966_router(&server.host_with_port()).await;

        let labels_product_ref_mock = server
            .mock("POST", "/labels")
            .match_request(|r| {
                let body = r.body().unwrap();
                let body_str = String::from_utf8(body.clone()).unwrap();

                body_str.contains("productRef(id:") && !body_str.contains("first:")
            })
            .expect(1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"
                {
                  "data": {
                    "productRef": {
                      "__typename": "Product",
                      "id": "p1"
                    }
                  }
                }
                "#,
            )
            .create();

        let product_entities_single_mock = server
            .mock("POST", "/products")
            .match_request(|r| {
                let body = r.body().unwrap();
                let body_str = String::from_utf8(body.clone()).unwrap();

                body_str.contains("_entities")
                    && body_str.contains("$representations")
                    && !body_str.contains("_e0")
            })
            .expect(1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"
                {
                  "data": {
                    "_entities": [
                      {
                        "metadata": {
                          "entity.root\t": "entity root value"
                        },
                        "renamedMetadata": {
                          "entity.root\t": "entity root value"
                        }
                      }
                    ]
                  }
                }
                "#,
            )
            .create();

        let res = router
            .send_graphql_request(
                r#"
                {
                  productRef(id: "p1") {
                    id
                    metadata
                    renamedMetadata: metadata
                  }
                }
                "#,
                None,
                None,
            )
            .await;

        labels_product_ref_mock.assert();
        product_entities_single_mock.assert();

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
        {
          "data": {
            "productRef": {
              "id": "p1",
              "metadata": {
                "entity.root\t": "entity root value"
              },
              "renamedMetadata": {
                "entity.root\t": "entity root value"
              }
            }
          }
        }
        "#);
    }

    #[ntex::test]
    /// https://github.com/graphql-hive/router/issues/966
    async fn issue_966_custom_scalar_batched_entity_fetch() {
        let mut server = mockito::Server::new_async().await;
        let router = build_issue_966_router(&server.host_with_port()).await;

        let labels_product_ref_batch_mock = server
            .mock("POST", "/labels")
            .match_request(|r| {
                let body = r.body().unwrap();
                let body_str = String::from_utf8(body.clone()).unwrap();

                body_str.contains("first: productRef(") && body_str.contains("second: productRef(")
            })
            .expect(1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"
                {
                  "data": {
                    "first": {
                      "__typename": "Product",
                      "id": "p1"
                    },
                    "second": {
                      "__typename": "Product",
                      "id": "p2"
                    }
                  }
                }
                "#,
            )
            .create();

        let product_entities_mock = server
            .mock("POST", "/products")
            .match_request(|r| {
                let body = r.body().unwrap();
                let body_str = String::from_utf8(body.clone()).unwrap();

                body_str.contains("_entities")
                    && body_str.contains("_e0")
                    && body_str.contains("_e1")
            })
            .expect(1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"
                {
                  "data": {
                    "_e0": [
                      {
                        "renamedMetadata": {
                          "batch.two\t": "second"
                        }
                      }
                    ],
                    "_e1": [
                      {
                        "metadata": {
                          "batch.one\t": "first"
                        }
                      }
                    ]
                  }
                }
                "#,
            )
            .create();

        let res = router
            .send_graphql_request(
                r#"
                {
                  first: productRef(id: "p1") {
                    metadata
                  }
                  second: productRef(id: "p2") {
                    renamedMetadata: metadata
                  }
                }
                "#,
                None,
                None,
            )
            .await;

        labels_product_ref_batch_mock.assert();
        product_entities_mock.assert();

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
        {
          "data": {
            "first": {
              "metadata": {
                "batch.one\t": "first"
              }
            },
            "second": {
              "renamedMetadata": {
                "batch.two\t": "second"
              }
            }
          }
        }
        "#);
    }

    #[ntex::test]
    // Reproduces a bug where batched entity fetches were built from nullable list
    // items without filtering out `null` values.
    async fn batch_fetch_skips_null_source_entities() {
        let mut server = mockito::Server::new_async().await;
        let host = server.host_with_port();

        let router = TestRouter::builder()
            .inline_config(format!(
                r#"
                  supergraph:
                    source: file
                    path: src/issues/supergraph.batch-null-entity.graphql
                  override_subgraph_urls:
                    subgraphs:
                      inventory:
                        url: "http://{host}/inventory"
                      products:
                        url: "http://{host}/products"
                  "#
            ))
            .build()
            .start()
            .await;

        let products_mock = server
            .mock("POST", "/products")
            .match_request(|r| {
                let body = r.body().unwrap();
                let body_str = std::str::from_utf8(&body).unwrap();

                body_str.contains("products") && body_str.contains("topProducts")
            })
            .expect(1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"
                {
                  "data": {
                    "products": [
                      { "__typename": "Product", "upc": "1", "price": 100, "weight": 1 },
                      { "__typename": "Product", "upc": "2", "price": 200, "weight": 2 },
                      null
                    ],
                    "topProducts": [
                      { "__typename": "Product", "upc": "3" }
                    ]
                  }
                }
                "#,
            )
            .create();

        let inventory_mock = server
            .mock("POST", "/inventory")
            .match_request(|r| {
                let body = r.body().unwrap();
                let body_str = std::str::from_utf8(&body).unwrap();

                body_str.contains("_entities")
                    && body_str.contains("_e0")
                    && body_str.contains("_e1")
            })
            .expect(1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"
                {
                  "data": {
                    "_e0": [
                      { "shippingEstimate": 10 },
                      { "shippingEstimate": 20 }
                    ],
                    "_e1": [
                      { "inStock": true }
                    ]
                  }
                }
                "#,
            )
            .create();

        let res = router
            .send_graphql_request(
                r#"
                {
                  products {
                    shippingEstimate
                  }
                  topProducts {
                    inStock
                  }
                }
                "#,
                None,
                None,
            )
            .await;

        products_mock.assert();
        inventory_mock.assert();

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
        {
          "data": {
            "products": [
              {
                "shippingEstimate": 10
              },
              {
                "shippingEstimate": 20
              },
              null
            ],
            "topProducts": [
              {
                "inStock": true
              }
            ]
          }
        }
        "#);
    }

    #[ntex::test]
    /// https://github.com/graphql-hive/router/issues/1099
    ///
    /// When an entity's `@key` is set to only `__typename` (use case is singleton that lives on another subgraph),
    /// them the executor must still execute the `_entities` fetch call to the subgraph.
    ///
    /// Previously the representation projection skipped `__typename`, leading to an empty representation,
    /// and no fetch call was made.
    /// This happened because `__typename` has special handling in the representation projection
    /// that bypassed the standard field projection logic.
    async fn issue_1099_entities_fetch_with_only_typename_key() {
        let mut server = mockito::Server::new_async().await;
        let host = server.host_with_port();

        let router = TestRouter::builder()
            .inline_config(format!(
                r#"
                  supergraph:
                    source: file
                    path: src/issues/supergraph.1099.graphql
                  query_planner:
                    allow_expose: true
                  override_subgraph_urls:
                    subgraphs:
                      warehouse:
                        url: "http://{host}/warehouse"
                      reviews:
                        url: "http://{host}/reviews"
                  "#
            ))
            .build()
            .start()
            .await;

        // Step 1: warehouse returns catalogEntry { __typename, sku }
        let warehouse_mock = server
            .mock("POST", "/warehouse")
            .match_request(|r| {
                let body = String::from_utf8(r.body().unwrap().clone()).unwrap();
                body.contains("catalogEntry")
            })
            .expect(1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                  "data": {
                    "catalogEntry": { "__typename": "CatalogEntry", "sku": "SKU-REPRO-001" }
                  }
                }"#,
            )
            .create();

        // Step 2: reviews must receive the _entities fetch built from a
        // representation whose only key field is __typename.
        let reviews_mock = server
            .mock("POST", "/reviews")
            .match_request(|r| {
                let body = String::from_utf8(r.body().unwrap().clone()).unwrap();
                body.contains("_entities") && body.contains("$representations")
            })
            .expect(1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                  "data": {
                    "_entities": [
                      { "__typename": "CatalogEntry", "rating": { "score": 42 } }
                    ]
                  }
                }"#,
            )
            .create();

        let res = router
            .send_graphql_request("{ catalogEntry { sku rating { score } } }", None, None)
            .await;

        warehouse_mock.assert();
        reviews_mock.assert();

        // The core thing here is `rating` field - if `__typename` is not written in projections, `rating` is `null`
        // becuase no fetch is made (and the previous assertion has failed)
        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
        {
          "data": {
            "catalogEntry": {
              "sku": "SKU-REPRO-001",
              "rating": {
                "score": 42
              }
            }
          }
        }
        "#);
    }

    #[ntex::test]
    /// Inline string arguments must be re-escaped per the GraphQL spec when
    /// the router emits the operation to a subgraph. A value such as
    /// `"\"quoted\""` is decoded to `"quoted"` while parsing the incoming
    /// operation; if it is re-emitted bare, the subgraph receives the invalid
    /// literal `payload: ""quoted""`.
    async fn escape_inline_string_arguments_for_subgraph() {
        let mut server = mockito::Server::new_async().await;
        let host = server.host_with_port();

        let router = TestRouter::builder()
            .inline_config(format!(
                r#"
                  supergraph:
                    source: file
                    path: src/issues/supergraph.escape-string-arguments.graphql
                  override_subgraph_urls:
                    subgraphs:
                      entries:
                        url: "http://{host}/entries"
                  "#
            ))
            .build()
            .start()
            .await;

        let entries_mock = server
            .mock("POST", "/entries")
            .match_request(|r| {
                use sonic_rs::JsonValueTrait;

                let body = r.body().unwrap();
                let body_str = String::from_utf8(body.clone()).unwrap();

                let parsed: sonic_rs::Value =
                    sonic_rs::from_slice(body).expect("subgraph body must be valid JSON");
                let query = parsed
                    .get(&"query")
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
                    .expect("subgraph body must contain a `query` string");

                let has_escaped = query.contains(r#"payload: "\"quoted\"""#);
                let has_unescaped = query.contains(r#"payload: ""quoted"""#);

                assert!(
                    has_escaped,
                    "expected escaped string literal in subgraph query, got: {body_str}"
                );
                assert!(
                    !has_unescaped,
                    "subgraph query must not contain unescaped quotes, got: {body_str}"
                );

                true
            })
            .expect(1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"
                {
                  "data": {
                    "writeEntry": { "id": "primary" }
                  }
                }
                "#,
            )
            .create();

        let res = router
            .send_graphql_request(
                r#"
                mutation {
                  writeEntry(
                    bucket: "primary"
                    attempt: 1
                    entries: [
                      {
                        upsert: {
                          schemaKey: "Entry"
                          attributes: [
                            { key: "field-1", payload: "\"quoted\"" }
                          ]
                        }
                      }
                    ]
                  ) {
                    id
                  }
                }
                "#,
                None,
                None,
            )
            .await;

        entries_mock.assert();

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
        {
          "data": {
            "writeEntry": {
              "id": "primary"
            }
          }
        }
        "#);
    }

    #[ntex::test]
    /// https://github.com/graphql-hive/router/issues/1154
    ///
    /// Per the GraphQL spec (Handling Field Errors), when a non-null field
    /// errors, the `null` must bubble up to the nearest nullable ancestor. For a
    /// non-null *root* field with no nullable ancestor, the `null` propagates all
    /// the way to `data`, so the response must be `"data": null`.
    async fn non_null_root_field_error_propagates_to_data() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let subgraphs_url = subgraphs.url();

        let router = TestRouter::builder()
            .inline_config(format!(
                r#"
                  supergraph:
                    source: file
                    path: src/issues/supergraph.non-null-root-field.graphql
                  override_subgraph_urls:
                    subgraphs:
                      accounts:
                        url: "{subgraphs_url}/accounts"
                  "#
            ))
            .build()
            .start()
            .await;

        // Nullable root field that errors: `null` stays on the field, `data` is
        // still an object.
        let nullable_res = router
            .send_graphql_request("{ nullableFieldThatErrors }", None, None)
            .await;
        assert!(nullable_res.status().is_success(), "Expected 200 OK");
        insta::assert_snapshot!(nullable_res.json_body_string_pretty().await, @r#"
        {
          "data": {
            "nullableFieldThatErrors": null
          },
          "errors": [
            {
              "message": "nullableFieldThatErrors always fails",
              "locations": [
                {
                  "line": 1,
                  "column": 2
                }
              ],
              "path": [
                "nullableFieldThatErrors"
              ],
              "extensions": {
                "code": "DOWNSTREAM_SERVICE_ERROR",
                "serviceName": "accounts"
              }
            }
          ]
        }
        "#);

        // Nested nullable field that errors: `null` stays on the nested field, `data` is
        // still an object.
        let nullable_nested_res = router
            .send_graphql_request("{ nullableNested { fieldThatErrors } }", None, None)
            .await;
        assert!(nullable_nested_res.status().is_success(), "Expected 200 OK");
        insta::assert_snapshot!(nullable_nested_res.json_body_string_pretty().await, @r#"
        {
          "data": {
            "nullableNested": {
              "fieldThatErrors": null
            }
          },
          "errors": [
            {
              "message": "NullableNested.fieldThatErrors always fails",
              "extensions": {
                "code": "DOWNSTREAM_SERVICE_ERROR",
                "serviceName": "accounts"
              }
            }
          ]
        }
        "#);

        // Non-null root field that errors: `null` must bubble all the way up to
        // `data`, so `data` itself becomes `null`.
        let non_null_res = router
            .send_graphql_request("{ nonNullFieldThatErrors }", None, None)
            .await;
        assert!(non_null_res.status().is_success(), "Expected 200 OK");
        insta::assert_snapshot!(non_null_res.json_body_string_pretty().await, @r#"
        {
          "data": null,
          "errors": [
            {
              "message": "nonNullFieldThatErrors always fails",
              "locations": [
                {
                  "line": 1,
                  "column": 2
                }
              ],
              "path": [
                "nonNullFieldThatErrors"
              ],
              "extensions": {
                "code": "DOWNSTREAM_SERVICE_ERROR",
                "serviceName": "accounts"
              }
            }
          ]
        }
        "#);

        // Non-null, nested field that errors: `null` must bubble all the way up.
        let non_null_res = router
            .send_graphql_request("{ nonNullNested { fieldThatErrors } }", None, None)
            .await;
        assert!(non_null_res.status().is_success(), "Expected 200 OK");
        insta::assert_snapshot!(non_null_res.json_body_string_pretty().await, @r#"
        {
          "data": null,
          "errors": [
            {
              "message": "NonNullNested.fieldThatErrors always fails",
              "locations": [
                {
                  "line": 1,
                  "column": 16
                }
              ],
              "path": [
                "nonNullNested",
                "fieldThatErrors"
              ],
              "extensions": {
                "code": "DOWNSTREAM_SERVICE_ERROR",
                "serviceName": "accounts"
              }
            }
          ]
        }
        "#);
    }

    #[ntex::test]
    /// Confirms error + null propagation lands at the nearest nullable position, per the
    /// GraphQL spec (Handling Field Errors), across three nullability chains.
    async fn error_and_null_propagation() {
        let mut server = mockito::Server::new_async().await;
        let host = server.host_with_port();

        let router = TestRouter::builder()
            .inline_config(format!(
                r#"
                  supergraph:
                    source: file
                    path: src/issues/supergraph.error-and-null-propagation.graphql
                  override_subgraph_urls:
                    subgraphs:
                      accounts:
                        url: "http://{host}/accounts"
                  "#
            ))
            .build()
            .start()
            .await;

        // Helper: a subgraph mock for one chain that returns `c: null` + a field error.
        let mut mock_chain = |chain: &str| {
            let chain = chain.to_string();
            server
                .mock("POST", "/accounts")
                .match_request(move |r| {
                    String::from_utf8(r.body().unwrap().clone())
                        .unwrap()
                        .contains(&chain)
                })
                .with_status(200)
                .with_header("content-type", "application/json")
        };

        let _m1 = mock_chain("chain1")
            .with_body(
                r#"{"data":{"chain1":{"b":{"c":null}}},
                    "errors":[{"message":"c failed","path":["chain1","b","c"]}]}"#,
            )
            .create();
        let _m2 = mock_chain("chain2")
            .with_body(
                r#"{"data":{"chain2":{"b":{"c":null}}},
                    "errors":[{"message":"c failed","path":["chain2","b","c"]}]}"#,
            )
            .create();
        let _m3 = mock_chain("chain3")
            .with_body(
                r#"{"data":{"chain3":{"b":{"c":null}}},
                    "errors":[{"message":"c failed","path":["chain3","b","c"]}]}"#,
            )
            .create();

        // Chain 1: nullable -> non-null -> nullable. The leaf `c` is nullable, so its
        // `null` stays put — nothing propagates.
        let res1 = router
            .send_graphql_request("{ chain1 { b { c } } }", None, None)
            .await;
        insta::assert_snapshot!(res1.json_body_string_pretty().await, @r#"
        {
          "data": {
            "chain1": {
              "b": {
                "c": null
              }
            }
          },
          "errors": [
            {
              "message": "c failed",
              "path": [
                "chain1",
                "b",
                "c"
              ],
              "extensions": {
                "code": "DOWNSTREAM_SERVICE_ERROR",
                "serviceName": "accounts"
              }
            }
          ]
        }
        "#);

        // Chain 2: non-null -> non-null -> non-null. The leaf `c` is Non-Null, so its
        // `null` bubbles up through every non-null level, all the way to `data`.
        let res2 = router
            .send_graphql_request("{ chain2 { b { c } } }", None, None)
            .await;
        insta::assert_snapshot!(res2.json_body_string_pretty().await, @r#"
        {
          "data": null,
          "errors": [
            {
              "message": "c failed",
              "path": [
                "chain2",
                "b",
                "c"
              ],
              "extensions": {
                "code": "DOWNSTREAM_SERVICE_ERROR",
                "serviceName": "accounts"
              }
            }
          ]
        }
        "#);

        // Chain 3: non-null -> nullable -> non-null. The leaf `c` is non-null and bubbles,
        // but stops at the nearest nullable ancestor `b`, preserving `chain3`/`data` as-is.
        let res3 = router
            .send_graphql_request("{ chain3 { b { c } } }", None, None)
            .await;
        insta::assert_snapshot!(res3.json_body_string_pretty().await, @r#"
        {
          "data": {
            "chain3": {
              "b": null
            }
          },
          "errors": [
            {
              "message": "c failed",
              "path": [
                "chain3",
                "b",
                "c"
              ],
              "extensions": {
                "code": "DOWNSTREAM_SERVICE_ERROR",
                "serviceName": "accounts"
              }
            }
          ]
        }
        "#);
    }

    #[ntex::test]
    /// https://github.com/graphql-hive/router/issues/1110
    ///
    /// When an entity is fails to resolve, then executor must not leave Non-Null fields as `null`.
    ///
    /// Here, `orders` provides only `user.id`; `users` returns `null` for the entity, so
    /// `User.email: String!` has no value.
    ///
    /// That `null` must go up through `user: User!` and `orders: [Order!]!`, collapsing `data` to `null`.
    async fn issue_1110_unresolved_entity_non_null_propagation() {
        let mut server = mockito::Server::new_async().await;
        let host = server.host_with_port();

        let router = TestRouter::builder()
            .inline_config(format!(
                r#"
                  supergraph:
                    source: file
                    path: src/issues/supergraph.1110.graphql
                  override_subgraph_urls:
                    subgraphs:
                      orders:
                        url: "http://{host}/orders"
                      users:
                        url: "http://{host}/users"
                  "#
            ))
            .build()
            .start()
            .await;

        // `orders` resolves the order and provides only the user's key.
        let _orders = server
            .mock("POST", "/orders")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data":{"orders":[{"id":"1","user":{"__typename":"User","id":"5"}}]}}"#)
            .create();

        // `users` cannot resolve the referenced entity, so it returns `null` for it.
        let _users = server
            .mock("POST", "/users")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data":{"_entities":[null]}}"#)
            .create();

        let res = router
            .send_graphql_request("{ orders { id user { id name email } } }", None, None)
            .await;
        assert!(res.status().is_success(), "Expected 200 OK");

        // `email: String!` is `null`, so it bubbles `email -> user -> Order`, then all the way up to `data`.
        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
        {
          "data": null
        }
        "#);
    }

    #[ntex::test]
    /// null propagation for nested lists.
    ///
    /// `grid: [[Int!]]` has a nullable outer
    /// element but a Non-Null inner element.
    ///
    /// A `null` inside an inner list bubbles to that inner list (non-null),
    /// but the outer list keeps it because its own element is nullable.
    async fn nested_list_null_propagation() {
        use crate::testkit::mock_subgraphs::mock_subgraphs;
        use serde_json::json;

        let subgraphs = TestSubgraphs::builder()
            .with_on_request(mock_subgraphs(json!({
                "accounts": {
                    "query": {
                        "grid": [[1, null], [2, 3]]
                    }
                }
            })))
            .build()
            .start()
            .await;

        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                  supergraph:
                    source: file
                    path: src/issues/supergraph.error-and-null-propagation.graphql
                  "#,
            )
            .build()
            .start()
            .await;

        let res = router.send_graphql_request("{ grid }", None, None).await;
        assert!(res.status().is_success(), "Expected 200 OK");
        // `[1, null]`: the `null` is a Non-Null inner element -> the inner list collapses
        // to `null`. The outer list's element is nullable, so that `null` stays in place.
        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
        {
          "data": {
            "grid": [
              null,
              [
                2,
                3
              ]
            ]
          }
        }
        "#);
    }

    #[ntex::test]
    /// A field error inside a fully Non-Null nested list (`[[Int!]!]!`) bubbles up through
    /// every level — the inner list, the outer list, and the Non-Null field — collapsing
    /// `data` itself.
    async fn nested_list_error_propagates_to_data() {
        let mut server = mockito::Server::new_async().await;
        let host = server.host_with_port();

        let router = TestRouter::builder()
            .inline_config(format!(
                r#"
                  supergraph:
                    source: file
                    path: src/issues/supergraph.error-and-null-propagation.graphql
                  override_subgraph_urls:
                    subgraphs:
                      accounts:
                        url: "http://{host}/accounts"
                  "#
            ))
            .build()
            .start()
            .await;

        // `matrix[0][1]` is `null` at a Non-Null `Int!`, reported with a field error.
        let _m = server
            .mock("POST", "/accounts")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"data":{"matrix":[[1,null]]},
                    "errors":[{"message":"matrix element failed","path":["matrix",0,1]}]}"#,
            )
            .create();

        let res = router.send_graphql_request("{ matrix }", None, None).await;
        assert!(res.status().is_success(), "Expected 200 OK");
        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
        {
          "data": null,
          "errors": [
            {
              "message": "matrix element failed",
              "path": [
                "matrix",
                0,
                1
              ],
              "extensions": {
                "code": "DOWNSTREAM_SERVICE_ERROR",
                "serviceName": "accounts"
              }
            }
          ]
        }
        "#);
    }

    #[ntex::test]
    /// A field typed `[[[Int]!]]`: the middle list's element is Non-Null while the outer
    /// list's element is nullable. A `null` middle element bubbles to its middle list, but
    /// the outer list keeps that `null` (its own element is nullable).
    async fn triple_nested_list_null_propagation() {
        use crate::testkit::mock_subgraphs::mock_subgraphs;
        use serde_json::json;

        let subgraphs = TestSubgraphs::builder()
            .with_on_request(mock_subgraphs(json!({
                "accounts": {
                    "query": {
                        // `[[1], null]`: the `null` is a Non-Null middle element (`[Int]!`).
                        "cube": [[[1], null], [[2]]]
                    }
                }
            })))
            .build()
            .start()
            .await;

        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                  supergraph:
                    source: file
                    path: src/issues/supergraph.error-and-null-propagation.graphql
                  "#,
            )
            .build()
            .start()
            .await;

        let res = router.send_graphql_request("{ cube }", None, None).await;
        assert!(res.status().is_success(), "Expected 200 OK");
        // The `null` bubbles to its middle list (`[[1], null]` -> `null`), but the outer
        // list keeps it because the outer element is nullable.
        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
        {
          "data": {
            "cube": [
              null,
              [
                [
                  2
                ]
              ]
            ]
          }
        }
        "#);
    }

    #[ntex::test]
    /// Null propagation through a Federation `_entities` fetch. `Product.measurements`
    /// (`[[Int!]!]!`) is resolved in the `metrics` subgraph; one entity comes back with a
    /// `null` inner element. After the entity merge, that `null` must bubble up through the
    /// nested Non-Null list to the whole `Product`. Because the `products` list element is
    /// nullable, the collapsed product becomes `null` and its sibling survives.
    async fn entity_nested_list_null_propagation() {
        use crate::testkit::mock_subgraphs::mock_subgraphs;
        use serde_json::json;

        let subgraphs = TestSubgraphs::builder()
            .with_on_request(mock_subgraphs(json!({
                "products": {
                    "query": {
                        "products": [
                            { "__typename": "Product", "id": "1" },
                            { "__typename": "Product", "id": "2" }
                        ]
                    }
                },
                "metrics": {
                    "entities": [
                        // Product 1 has a `null` at a Non-Null inner element (`Int!`).
                        { "__typename": "Product", "id": "1", "measurements": [[1, null]] },
                        { "__typename": "Product", "id": "2", "measurements": [[2, 3]] }
                    ]
                }
            })))
            .build()
            .start()
            .await;

        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                  supergraph:
                    source: file
                    path: src/issues/supergraph.entity-list-propagation.graphql
                  "#,
            )
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request("{ products { id measurements } }", None, None)
            .await;
        assert!(res.status().is_success(), "Expected 200 OK");
        // Product 1's `null` bubbles `Int! -> inner list -> outer list -> measurements ->
        // Product`, collapsing it to `null` (kept, since the list element is nullable).
        // Product 2 is unaffected.
        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
        {
          "data": {
            "products": [
              null,
              {
                "id": "2",
                "measurements": [
                  [
                    2,
                    3
                  ]
                ]
              }
            ]
          }
        }
        "#);

        let res = router
            .send_graphql_request("{ productsNoNulls { id measurements } }", None, None)
            .await;
        assert!(res.status().is_success(), "Expected 200 OK");
        // Product 1's `null` bubbles `Int! -> inner list -> outer list -> measurements ->
        // Product`, collapsing it to `null` (kept, since the list element is nullable).
        // Product 2 is unaffected.
        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
        {
          "data": null
        }
        "#);
    }

    #[ntex::test]
    async fn union_shared_field_different_selections() {
        let mut server = mockito::Server::new_async().await;
        let host = server.host_with_port();

        let router = TestRouter::builder()
            .inline_config(format!(
                r#"
                  supergraph:
                    source: file
                    path: src/issues/supergraph.union-shared-field.graphql
                  override_subgraph_urls:
                    subgraphs:
                      a:
                        url: "http://{host}/a"
                  "#
            ))
            .build()
            .start()
            .await;

        let _a = server
            .mock("POST", "/a")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"data":{"payloads":[
                    {"__typename":"ApprovalFlowCreated","order":{"id":"1"}},
                    {"__typename":"ApprovalFlowCompleted","order":{"id":"2"}}
                ]}}"#,
            )
            .create();

        let res = router
            .send_graphql_request(
                r#"{
                  payloads {
                    __typename
                    ... on ApprovalFlowCreated { order { id } }
                    ... on ApprovalFlowRejected { order { id version orderNumber } }
                    ... on ApprovalFlowCompleted { order { id } }
                  }
                }"#,
                None,
                None,
            )
            .await;
        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
        {
          "data": {
            "payloads": [
              {
                "__typename": "ApprovalFlowCreated",
                "order": {
                  "id": "1"
                }
              },
              {
                "__typename": "ApprovalFlowCompleted",
                "order": {
                  "id": "2"
                }
              }
            ]
          }
        }
        "#);
    }

    #[ntex::test]
    /// https://github.com/graphql-hive/router/pull/1166
    ///
    /// A union (`SearchResult`) whose members share the response key `chain` but select
    /// different sub-fields per branch: `Collection`/`CurrencyV2` pick `chain { identifier }`
    /// while `Item` picks `chain { arch }` (and `Chain.arch` is Non-Null). The `arch`
    /// selection must NOT leak into the `Collection`/`CurrencyV2` projections — otherwise
    /// its missing value bubbles up (Non-Null) and collapses the response to `data: null`.
    async fn issue_1166_disjoint_fragments_shared_field() {
        let mut server = mockito::Server::new_async().await;
        let host = server.host_with_port();

        let router = TestRouter::builder()
            .inline_config(format!(
                r#"
                  supergraph:
                    source: file
                    path: src/issues/supergraph.1166-disjoint-fragments.graphql
                  override_subgraph_urls:
                    subgraphs:
                      a:
                        url: "http://{host}/a"
                  "#
            ))
            .build()
            .start()
            .await;

        let _a = server
            .mock("POST", "/a")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"data":{"searchUniversal":[
                    {"__typename":"Collection","chain":{"identifier":"c1"}},
                    {"__typename":"CurrencyV2","chain":{"identifier":"cur1"}},
                    {"__typename":"Item","chain":{"arch":"x86"}}
                ]}}"#,
            )
            .create();

        let res = router
            .send_graphql_request(
                r#"{
                  searchUniversal {
                    __typename
                    ... on Collection { chain { identifier } }
                    ... on CurrencyV2 { chain { identifier } }
                    ... on Item { chain { arch } }
                  }
                }"#,
                None,
                None,
            )
            .await;
        assert!(res.status().is_success(), "Expected 200 OK");
        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
        {
          "data": {
            "searchUniversal": [
              {
                "__typename": "Collection",
                "chain": {
                  "identifier": "c1"
                }
              },
              {
                "__typename": "CurrencyV2",
                "chain": {
                  "identifier": "cur1"
                }
              },
              {
                "__typename": "Item",
                "chain": {
                  "arch": "x86"
                }
              }
            ]
          }
        }
        "#);
    }

    #[ntex::test]
    /// Overlapping (not disjoint) type guards sharing a response key. `meta` is selected
    /// under `... on Article` (`{ title }`) and under the interface `... on HasMeta`
    /// (`{ wordCount }`); the guards overlap on `Article`. The `Article`-only `title`
    /// (Non-Null) must NOT leak into the `Video` branch — otherwise its missing value
    /// bubbles and collapses the response.
    async fn interface_overlapping_guards_shared_field() {
        let mut server = mockito::Server::new_async().await;
        let host = server.host_with_port();

        let router = TestRouter::builder()
            .inline_config(format!(
                r#"
                  supergraph:
                    source: file
                    path: src/issues/supergraph.interface-shared-field.graphql
                  override_subgraph_urls:
                    subgraphs:
                      a:
                        url: "http://{host}/a"
                  "#
            ))
            .build()
            .start()
            .await;

        let _a = server
            .mock("POST", "/a")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"data":{"search":[
                    {"__typename":"Article","meta":{"title":"T","wordCount":100}},
                    {"__typename":"Video","meta":{"wordCount":50}}
                ],"searchMeta":[
                    {"__typename":"Article","meta":{"title":"T","wordCount":100}},
                    {"__typename":"Video","meta":{"wordCount":50}}
                ]}}"#,
            )
            .create();

        let res = router
            .send_graphql_request(
                r#"{
                  search {
                    __typename
                    ... on Article { meta { title } }
                    ... on HasMeta { meta { wordCount } }
                  }
                  searchMeta {
                    __typename
                    ... on Article { meta { title } }
                    ... on HasMeta { meta { wordCount } }
                  }
                }"#,
                None,
                None,
            )
            .await;
        assert!(res.status().is_success(), "Expected 200 OK");
        // Both fields scope `meta` per concrete type: `Article` (in both the `Article` and
        // `HasMeta` scopes) gets `{ title, wordCount }`, `Video` (only `HasMeta`) gets just
        // `{ wordCount }` — `title` never leaks. `searchMeta` (interface parent, where
        // `... on HasMeta` inlines to an implicit "any type" guard) resolves the same way.
        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
        {
          "data": {
            "search": [
              {
                "__typename": "Article",
                "meta": {
                  "title": "T",
                  "wordCount": 100
                }
              },
              {
                "__typename": "Video",
                "meta": {
                  "wordCount": 50
                }
              }
            ],
            "searchMeta": [
              {
                "__typename": "Article",
                "meta": {
                  "title": "T",
                  "wordCount": 100
                }
              },
              {
                "__typename": "Video",
                "meta": {
                  "wordCount": 50
                }
              }
            ]
          }
        }
        "#);
    }

    #[ntex::test]
    /// Nested abstract types: an interface field (`Book.related: Item`) that itself returns
    /// an interface, with fragments at both levels. Each level must resolve and scope by the
    /// concrete `__typename` — the inner `Movie` must get `runtime` (not the sibling
    /// `... on Book { title }`).
    async fn nested_abstract_type_resolution() {
        let mut server = mockito::Server::new_async().await;
        let host = server.host_with_port();

        let router = TestRouter::builder()
            .inline_config(format!(
                r#"
                  supergraph:
                    source: file
                    path: src/issues/supergraph.nested-abstract.graphql
                  override_subgraph_urls:
                    subgraphs:
                      a:
                        url: "http://{host}/a"
                  "#
            ))
            .build()
            .start()
            .await;

        let _a = server
            .mock("POST", "/a")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"data":{"items":[
                    {"__typename":"Book","id":"b1","title":"GoT",
                     "related":{"__typename":"Movie","id":"m1","runtime":120}},
                    {"__typename":"Movie","id":"m2","runtime":90}
                ]}}"#,
            )
            .create();

        let res = router
            .send_graphql_request(
                r#"{
                  items {
                    __typename
                    ... on Book {
                      title
                      related {
                        __typename
                        ... on Book { title }
                        ... on Movie { runtime }
                      }
                    }
                    ... on Movie { runtime }
                  }
                }"#,
                None,
                None,
            )
            .await;
        assert!(res.status().is_success(), "Expected 200 OK");
        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
        {
          "data": {
            "items": [
              {
                "__typename": "Book",
                "title": "GoT",
                "related": {
                  "__typename": "Movie",
                  "runtime": 120
                }
              },
              {
                "__typename": "Movie",
                "runtime": 90
              }
            ]
          }
        }
        "#);
    }

    #[ntex::test]
    /// A fragment that validates as a possible spread but does NOT apply to the runtime type:
    /// `... on Movie { special }` (`special: String!`) against a `Book` object. Its fields —
    /// including the Non-Null `special` — must be silently skipped, NOT treated as a missing
    /// Non-Null that bubbles. The `Book` must survive.
    async fn fragment_validates_but_does_not_apply() {
        let mut server = mockito::Server::new_async().await;
        let host = server.host_with_port();

        let router = TestRouter::builder()
            .inline_config(format!(
                r#"
                  supergraph:
                    source: file
                    path: src/issues/supergraph.nested-abstract.graphql
                  override_subgraph_urls:
                    subgraphs:
                      a:
                        url: "http://{host}/a"
                  "#
            ))
            .build()
            .start()
            .await;

        let _a = server
            .mock("POST", "/a")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"data":{"items":[
                    {"__typename":"Book","id":"b1"},
                    {"__typename":"Movie","id":"m2","special":"x"}
                ]}}"#,
            )
            .create();

        let res = router
            .send_graphql_request(
                r#"{
                  items {
                    __typename
                    id
                    ... on Movie { special }
                  }
                }"#,
                None,
                None,
            )
            .await;
        assert!(res.status().is_success(), "Expected 200 OK");
        // `Book` doesn't match `... on Movie`, so `special` (Non-Null) is not collected for it
        // and must not bubble — the `Book` survives with just its interface fields.
        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
        {
          "data": {
            "items": [
              {
                "__typename": "Book",
                "id": "b1"
              },
              {
                "__typename": "Movie",
                "id": "m2",
                "special": "x"
              }
            ]
          }
        }
        "#);
    }

    #[ntex::test]
    /// Distributed union: subgraph A owns `feed` and resolves each member's `__typename` + key
    /// (`id`); subgraph B owns the type-specific fields (`Photo.caption`, `Video.duration`).
    /// The router must issue per-concrete-type `_entities` fetches to B and merge them back by
    /// `__typename`, so each member gets exactly its own fields.
    async fn distributed_union_per_type_entity_fetch() {
        use crate::testkit::mock_subgraphs::mock_subgraphs;
        use serde_json::json;

        let subgraphs = TestSubgraphs::builder()
            .with_on_request(mock_subgraphs(json!({
                "a": {
                    "query": {
                        "feed": [
                            { "__typename": "Photo", "id": "p1" },
                            { "__typename": "Video", "id": "v1" },
                            { "__typename": "Photo", "id": "p2" }
                        ]
                    }
                },
                "b": {
                    "entities": [
                        { "__typename": "Photo", "id": "p1", "caption": "sunset" },
                        { "__typename": "Video", "id": "v1", "duration": 120 },
                        { "__typename": "Photo", "id": "p2", "caption": "beach" }
                    ]
                }
            })))
            .build()
            .start()
            .await;

        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                  supergraph:
                    source: file
                    path: src/issues/supergraph.distributed-union.graphql
                  "#,
            )
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(
                r#"{
                  feed {
                    __typename
                    ... on Photo { caption }
                    ... on Video { duration }
                  }
                }"#,
                None,
                None,
            )
            .await;
        assert!(res.status().is_success(), "Expected 200 OK");
        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
        {
          "data": {
            "feed": [
              {
                "__typename": "Photo",
                "caption": "sunset"
              },
              {
                "__typename": "Video",
                "duration": 120
              },
              {
                "__typename": "Photo",
                "caption": "beach"
              }
            ]
          }
        }
        "#);
    }

    #[ntex::test]
    /// Abstract-type entities with a Non-Null field erroring in one member. Subgraph B's
    /// `_entities` returns `null` for `Photo.caption` (`String!`) on one entity. That `null`
    /// must bubble up through the Non-Null leaf, collapsing the `Photo`. Because `feedNullable`
    /// has a nullable element (`[Media]!`), the collapsed member survives as `null` and its
    /// siblings (including the other `Photo`) remain intact.
    async fn abstract_entities_nonnull_error_propagation() {
        use crate::testkit::mock_subgraphs::mock_subgraphs;
        use serde_json::json;

        let subgraphs = TestSubgraphs::builder()
            .with_on_request(mock_subgraphs(json!({
                "a": {
                    "query": {
                        "feedNullable": [
                            { "__typename": "Photo", "id": "p1" },
                            { "__typename": "Video", "id": "v1" },
                            { "__typename": "Photo", "id": "p2" }
                        ]
                    }
                },
                "b": {
                    "entities": [
                        // p1's Non-Null `caption` comes back `null` -> the Photo collapses.
                        { "__typename": "Photo", "id": "p1", "caption": null },
                        { "__typename": "Video", "id": "v1", "duration": 120 },
                        { "__typename": "Photo", "id": "p2", "caption": "beach" }
                    ]
                }
            })))
            .build()
            .start()
            .await;

        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                  supergraph:
                    source: file
                    path: src/issues/supergraph.distributed-union.graphql
                  "#,
            )
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(
                r#"{
                  feedNullable {
                    __typename
                    ... on Photo { caption }
                    ... on Video { duration }
                  }
                }"#,
                None,
                None,
            )
            .await;
        assert!(res.status().is_success(), "Expected 200 OK");
        // p1's `null` caption bubbles to the Photo; nullable list element keeps it as `null`.
        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
        {
          "data": {
            "feedNullable": [
              null,
              {
                "__typename": "Video",
                "duration": 120
              },
              {
                "__typename": "Photo",
                "caption": "beach"
              }
            ]
          }
        }
        "#);
    }
}
