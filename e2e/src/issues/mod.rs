#[cfg(test)]
mod issues_e2e_tests {
    use crate::testkit::{ClientResponseExt, Started, TestRouter};

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
}
