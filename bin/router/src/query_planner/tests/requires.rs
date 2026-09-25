use crate::query_planner::{
    tests::testkit::{build_query_plan_with_defaults, init_logger},
    utils::parsing::parse_operation,
};
use std::error::Error;

#[test]
fn two_same_service_calls_with_args_conflicts() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          products {
            isExpensive # price(withDiscount: false)
            reducedPrice # price(withDiscount: true)
          }
        }"#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/two-same-service-calls.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "inventory") {
          {
            products {
              __typename
              upc
            }
          }
        },
        Flatten(path: "products.@") {
          Fetch(service: "products") {
            {
              ... on Product {
                __typename
                upc
              }
            } =>
            {
              ... on Product {
                price(withDiscount: true)
                _internal_qp_alias_0: price(withDiscount: false)
              }
            }
          },
        },
        BatchFetch(service: "inventory") {
          {
            _e0 {
              paths: [
                "products.@"
              ]
              {
                ... on Product {
                  __typename
                  price: _internal_qp_alias_0
                  upc
                }
              }
            }
            _e1 {
              paths: [
                "products.@"
              ]
              {
                ... on Product {
                  __typename
                  price
                  upc
                }
              }
            }
          }
          {
            _e0: _entities(representations: $__batch_reps_0) {
              ... on Product {
                reducedPrice
              }
            }
            _e1: _entities(representations: $__batch_reps_1) {
              ... on Product {
                isExpensive
              }
            }
          }
        },
      },
    },
    "#);
    insta::assert_snapshot!(format!("{}", sonic_rs::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Sequence",
        "nodes": [
          {
            "kind": "Fetch",
            "serviceName": "inventory",
            "operationKind": "query",
            "operation": "{products{__typename upc}}"
          },
          {
            "kind": "Flatten",
            "path": [
              {
                "Field": "products"
              },
              "@"
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "products",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Product{price(withDiscount: true) _internal_qp_alias_0: price(withDiscount: false)}}}",
              "requires": [
                {
                  "kind": "InlineFragment",
                  "typeCondition": "Product",
                  "selections": [
                    {
                      "kind": "Field",
                      "name": "__typename"
                    },
                    {
                      "kind": "Field",
                      "name": "upc"
                    }
                  ]
                }
              ]
            }
          },
          {
            "kind": "BatchFetch",
            "serviceName": "inventory",
            "operationKind": "query",
            "operation": "query($__batch_reps_0:[_Any!]!, $__batch_reps_1:[_Any!]!){_e0: _entities(representations: $__batch_reps_0){...on Product{reducedPrice}} _e1: _entities(representations: $__batch_reps_1){...on Product{isExpensive}}}",
            "entityBatch": {
              "aliases": [
                {
                  "alias": "_e0",
                  "representationsVariableName": "__batch_reps_0",
                  "paths": [
                    [
                      {
                        "Field": "products"
                      },
                      "@"
                    ]
                  ],
                  "requires": [
                    {
                      "kind": "InlineFragment",
                      "typeCondition": "Product",
                      "selections": [
                        {
                          "kind": "Field",
                          "name": "__typename"
                        },
                        {
                          "kind": "Field",
                          "name": "_internal_qp_alias_0",
                          "alias": "price"
                        },
                        {
                          "kind": "Field",
                          "name": "upc"
                        }
                      ]
                    }
                  ]
                },
                {
                  "alias": "_e1",
                  "representationsVariableName": "__batch_reps_1",
                  "paths": [
                    [
                      {
                        "Field": "products"
                      },
                      "@"
                    ]
                  ],
                  "requires": [
                    {
                      "kind": "InlineFragment",
                      "typeCondition": "Product",
                      "selections": [
                        {
                          "kind": "Field",
                          "name": "__typename"
                        },
                        {
                          "kind": "Field",
                          "name": "price"
                        },
                        {
                          "kind": "Field",
                          "name": "upc"
                        }
                      ]
                    }
                  ]
                }
              ]
            }
          }
        ]
      }
    }
    "#);
    Ok(())
}

#[test]
fn two_same_service_calls() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          products {
            isExpensive
          }
        }"#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/two-same-service-calls.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "inventory") {
          {
            products {
              __typename
              upc
            }
          }
        },
        Flatten(path: "products.@") {
          Fetch(service: "products") {
            {
              ... on Product {
                __typename
                upc
              }
            } =>
            {
              ... on Product {
                price(withDiscount: true)
              }
            }
          },
        },
        Flatten(path: "products.@") {
          Fetch(service: "inventory") {
            {
              ... on Product {
                __typename
                price
                upc
              }
            } =>
            {
              ... on Product {
                isExpensive
              }
            }
          },
        },
      },
    },
    "#);
    insta::assert_snapshot!(format!("{}", sonic_rs::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Sequence",
        "nodes": [
          {
            "kind": "Fetch",
            "serviceName": "inventory",
            "operationKind": "query",
            "operation": "{products{__typename upc}}"
          },
          {
            "kind": "Flatten",
            "path": [
              {
                "Field": "products"
              },
              "@"
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "products",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Product{price(withDiscount: true)}}}",
              "requires": [
                {
                  "kind": "InlineFragment",
                  "typeCondition": "Product",
                  "selections": [
                    {
                      "kind": "Field",
                      "name": "__typename"
                    },
                    {
                      "kind": "Field",
                      "name": "upc"
                    }
                  ]
                }
              ]
            }
          },
          {
            "kind": "Flatten",
            "path": [
              {
                "Field": "products"
              },
              "@"
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "inventory",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Product{isExpensive}}}",
              "requires": [
                {
                  "kind": "InlineFragment",
                  "typeCondition": "Product",
                  "selections": [
                    {
                      "kind": "Field",
                      "name": "__typename"
                    },
                    {
                      "kind": "Field",
                      "name": "price"
                    },
                    {
                      "kind": "Field",
                      "name": "upc"
                    }
                  ]
                }
              ]
            }
          }
        ]
      }
    }
    "#);
    Ok(())
}

#[test]
fn simplest_requires() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          products {
            isExpensive
          }
        }"#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/simplest-requires.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "products") {
          {
            products {
              __typename
              upc
              price
            }
          }
        },
        Flatten(path: "products.@") {
          Fetch(service: "inventory") {
            {
              ... on Product {
                __typename
                price
                upc
              }
            } =>
            {
              ... on Product {
                isExpensive
              }
            }
          },
        },
      },
    },
    "#);
    insta::assert_snapshot!(format!("{}", sonic_rs::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Sequence",
        "nodes": [
          {
            "kind": "Fetch",
            "serviceName": "products",
            "operationKind": "query",
            "operation": "{products{__typename upc price}}"
          },
          {
            "kind": "Flatten",
            "path": [
              {
                "Field": "products"
              },
              "@"
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "inventory",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Product{isExpensive}}}",
              "requires": [
                {
                  "kind": "InlineFragment",
                  "typeCondition": "Product",
                  "selections": [
                    {
                      "kind": "Field",
                      "name": "__typename"
                    },
                    {
                      "kind": "Field",
                      "name": "price"
                    },
                    {
                      "kind": "Field",
                      "name": "upc"
                    }
                  ]
                }
              ]
            }
          }
        ]
      }
    }
    "#);
    Ok(())
}

#[test]
fn simplest_requires_with_local_sibling() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          products {
            isExpensive
            isAvailable
          }
        }"#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/requires-local-sibling.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "products") {
          {
            products {
              __typename
              upc
              price
            }
          }
        },
        Flatten(path: "products.@") {
          Fetch(service: "inventory") {
            {
              ... on Product {
                __typename
                price
                upc
              }
            } =>
            {
              ... on Product {
                isExpensive
                isAvailable
              }
            }
          },
        },
      },
    },
    "#);
    insta::assert_snapshot!(format!("{}", sonic_rs::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Sequence",
        "nodes": [
          {
            "kind": "Fetch",
            "serviceName": "products",
            "operationKind": "query",
            "operation": "{products{__typename upc price}}"
          },
          {
            "kind": "Flatten",
            "path": [
              {
                "Field": "products"
              },
              "@"
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "inventory",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Product{isExpensive isAvailable}}}",
              "requires": [
                {
                  "kind": "InlineFragment",
                  "typeCondition": "Product",
                  "selections": [
                    {
                      "kind": "Field",
                      "name": "__typename"
                    },
                    {
                      "kind": "Field",
                      "name": "price"
                    },
                    {
                      "kind": "Field",
                      "name": "upc"
                    }
                  ]
                }
              ]
            }
          }
        ]
      }
    }
    "#);
    Ok(())
}

#[test]
fn simple_requires() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          products {
            shippingEstimate
          }
        }"#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/simple-requires.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "products") {
          {
            products {
              __typename
              upc
              price
              weight
            }
          }
        },
        Flatten(path: "products.@") {
          Fetch(service: "inventory") {
            {
              ... on Product {
                __typename
                price
                weight
                upc
              }
            } =>
            {
              ... on Product {
                shippingEstimate
              }
            }
          },
        },
      },
    },
    "#);
    insta::assert_snapshot!(format!("{}", sonic_rs::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Sequence",
        "nodes": [
          {
            "kind": "Fetch",
            "serviceName": "products",
            "operationKind": "query",
            "operation": "{products{__typename upc price weight}}"
          },
          {
            "kind": "Flatten",
            "path": [
              {
                "Field": "products"
              },
              "@"
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "inventory",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Product{shippingEstimate}}}",
              "requires": [
                {
                  "kind": "InlineFragment",
                  "typeCondition": "Product",
                  "selections": [
                    {
                      "kind": "Field",
                      "name": "__typename"
                    },
                    {
                      "kind": "Field",
                      "name": "price"
                    },
                    {
                      "kind": "Field",
                      "name": "weight"
                    },
                    {
                      "kind": "Field",
                      "name": "upc"
                    }
                  ]
                }
              ]
            }
          }
        ]
      }
    }
    "#);
    Ok(())
}

#[test]
fn two_fields_same_subgraph_same_requirement() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          products {
            shippingEstimate
            shippingEstimate2
          }
        }"#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/two_fields_same_subgraph_same_requirement.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "products") {
          {
            products {
              __typename
              upc
              price
              weight
            }
          }
        },
        Flatten(path: "products.@") {
          Fetch(service: "inventory") {
            {
              ... on Product {
                __typename
                price
                weight
                upc
              }
            } =>
            {
              ... on Product {
                shippingEstimate2
                shippingEstimate
              }
            }
          },
        },
      },
    },
    "#);
    insta::assert_snapshot!(format!("{}", sonic_rs::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Sequence",
        "nodes": [
          {
            "kind": "Fetch",
            "serviceName": "products",
            "operationKind": "query",
            "operation": "{products{__typename upc price weight}}"
          },
          {
            "kind": "Flatten",
            "path": [
              {
                "Field": "products"
              },
              "@"
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "inventory",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Product{shippingEstimate2 shippingEstimate}}}",
              "requires": [
                {
                  "kind": "InlineFragment",
                  "typeCondition": "Product",
                  "selections": [
                    {
                      "kind": "Field",
                      "name": "__typename"
                    },
                    {
                      "kind": "Field",
                      "name": "price"
                    },
                    {
                      "kind": "Field",
                      "name": "weight"
                    },
                    {
                      "kind": "Field",
                      "name": "upc"
                    }
                  ]
                }
              ]
            }
          }
        ]
      }
    }
    "#);
    Ok(())
}

#[test]
fn simple_requires_with_child() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          products {
            shippingEstimate {
              price
            }
          }
        }"#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/simple_requires_with_child.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "products") {
          {
            products {
              __typename
              upc
              price
              weight
            }
          }
        },
        Flatten(path: "products.@") {
          Fetch(service: "inventory") {
            {
              ... on Product {
                __typename
                price
                weight
                upc
              }
            } =>
            {
              ... on Product {
                shippingEstimate {
                  price
                }
              }
            }
          },
        },
      },
    },
    "#);
    insta::assert_snapshot!(format!("{}", sonic_rs::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Sequence",
        "nodes": [
          {
            "kind": "Fetch",
            "serviceName": "products",
            "operationKind": "query",
            "operation": "{products{__typename upc price weight}}"
          },
          {
            "kind": "Flatten",
            "path": [
              {
                "Field": "products"
              },
              "@"
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "inventory",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Product{shippingEstimate{price}}}}",
              "requires": [
                {
                  "kind": "InlineFragment",
                  "typeCondition": "Product",
                  "selections": [
                    {
                      "kind": "Field",
                      "name": "__typename"
                    },
                    {
                      "kind": "Field",
                      "name": "price"
                    },
                    {
                      "kind": "Field",
                      "name": "weight"
                    },
                    {
                      "kind": "Field",
                      "name": "upc"
                    }
                  ]
                }
              ]
            }
          }
        ]
      }
    }
    "#);
    Ok(())
}

#[test]
fn keys_mashup() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          b {
            id
            a {
              id
              name
              nameInB
            }
          }
        }"#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/tests/keys-mashup.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "b") {
          {
            b {
              id
              a {
                __typename
                id
                compositeId {
                  two
                  three
                }
              }
            }
          }
        },
        Flatten(path: "b.a.@") {
          Fetch(service: "a") {
            {
              ... on A {
                __typename
                id
              }
            } =>
            {
              ... on A {
                name
              }
            }
          },
        },
        Flatten(path: "b.a.@") {
          Fetch(service: "b") {
            {
              ... on A {
                __typename
                name
                id
                compositeId {
                  two
                  three
                }
              }
            } =>
            {
              ... on A {
                nameInB
              }
            }
          },
        },
      },
    },
    "#);
    insta::assert_snapshot!(format!("{}", sonic_rs::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Sequence",
        "nodes": [
          {
            "kind": "Fetch",
            "serviceName": "b",
            "operationKind": "query",
            "operation": "{b{id a{__typename id compositeId{two three}}}}"
          },
          {
            "kind": "Flatten",
            "path": [
              {
                "Field": "b"
              },
              {
                "Field": "a"
              },
              "@"
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "a",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on A{name}}}",
              "requires": [
                {
                  "kind": "InlineFragment",
                  "typeCondition": "A",
                  "selections": [
                    {
                      "kind": "Field",
                      "name": "__typename"
                    },
                    {
                      "kind": "Field",
                      "name": "id"
                    }
                  ]
                }
              ]
            }
          },
          {
            "kind": "Flatten",
            "path": [
              {
                "Field": "b"
              },
              {
                "Field": "a"
              },
              "@"
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "b",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on A{nameInB}}}",
              "requires": [
                {
                  "kind": "InlineFragment",
                  "typeCondition": "A",
                  "selections": [
                    {
                      "kind": "Field",
                      "name": "__typename"
                    },
                    {
                      "kind": "Field",
                      "name": "name"
                    },
                    {
                      "kind": "Field",
                      "name": "id"
                    },
                    {
                      "kind": "Field",
                      "name": "compositeId",
                      "selections": [
                        {
                          "kind": "Field",
                          "name": "two"
                        },
                        {
                          "kind": "Field",
                          "name": "three"
                        }
                      ]
                    }
                  ]
                }
              ]
            }
          }
        ]
      }
    }
    "#);
    Ok(())
}

#[test]
fn deep_requires() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          feed {
            author {
              id
            }
          }
        }"#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/tests/deep-requires.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          {
            feed {
              __typename
              id
            }
          }
        },
        Flatten(path: "feed.@") {
          Fetch(service: "b") {
            {
              ... on Post {
                __typename
                id
              }
            } =>
            {
              ... on Post {
                comments(limit: 3) {
                  __typename
                  id
                }
              }
            }
          },
        },
        Flatten(path: "feed.@.comments.@") {
          Fetch(service: "a") {
            {
              ... on Comment {
                __typename
                id
              }
            } =>
            {
              ... on Comment {
                authorId
              }
            }
          },
        },
        Flatten(path: "feed.@") {
          Fetch(service: "b") {
            {
              ... on Post {
                __typename
                comments {
                  authorId
                }
                id
              }
            } =>
            {
              ... on Post {
                author {
                  id
                }
              }
            }
          },
        },
      },
    },
    "#);

    Ok(())
}

#[test]
/// related: https://github.com/graphql-hive/router/issues/1070
///
/// this test confirms that a re-entry `_entities`
fn requires_reentry_selects_entity_typename() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          productSearchResult(partialCriteria: "partialCriteria") {
            bestOffer {
              price
            }
          }
        }"#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/nested-key-requires-reentry.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "subgraph-a") {
          {
            productSearchResult(partialCriteria: "partialCriteria") {
              __typename
              searchCriteria {
                partialCriteria
                __typename
              }
            }
          }
        },
        Flatten(path: "productSearchResult.searchCriteria") {
          Fetch(service: "subgraph-b") {
            {
              ... on ProductSearchCriteria {
                __typename
                partialCriteria
              }
            } =>
            {
              ... on ProductSearchCriteria {
                fullCriteria
              }
            }
          },
        },
        Flatten(path: "productSearchResult") {
          Fetch(service: "subgraph-a") {
            {
              ... on ProductSearchResultResponse {
                __typename
                searchCriteria {
                  fullCriteria
                  partialCriteria
                }
              }
            } =>
            {
              ... on ProductSearchResultResponse {
                bestOffer {
                  price
                }
              }
            }
          },
        },
      },
    },
    "#);

    Ok(())
}

/// `tax` requires `price` from `pricing`, so `products` is called again through `_entities`.
/// `products` can select both its own key `id` and `pricing`'s bigger key `sku region`, but only
/// `id` is a key there. The representations it gets must use `id`.
#[test]
fn requires_reentry_uses_a_key_of_its_own_subgraph() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        {
          products {
            tax
          }
        }"#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/requires-reentry-own-key.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "products") {
          {
            products {
              __typename
              id
              sku
              region
            }
          }
        },
        Flatten(path: "products.@") {
          Fetch(service: "pricing") {
            {
              ... on Product {
                __typename
                sku
                region
              }
            } =>
            {
              ... on Product {
                price
              }
            }
          },
        },
        Flatten(path: "products.@") {
          Fetch(service: "products") {
            {
              ... on Product {
                __typename
                price
                id
              }
            } =>
            {
              ... on Product {
                tax
              }
            }
          },
        },
      },
    },
    "#);

    Ok(())
}

/// related: https://github.com/graphql-hive/router/issues/1539
///
/// `Order.name` requires `grade`, and `grade` requires `sku`.
/// Each of them sits on an `Order` that came out of an `_entities` fetch,
/// so each required field should be added to the fetch that already resolves its parent.
#[test]
fn chained_requires_after_entity_hop() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        {
          user {
            orders {
              name
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/chained-requires-after-hop.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "users") {
          {
            user {
              __typename
              id
            }
          }
        },
        Flatten(path: "user") {
          Fetch(service: "orders") {
            {
              ... on User {
                __typename
                id
              }
            } =>
            {
              ... on User {
                orders {
                  __typename
                  id
                  sku
                }
              }
            }
          },
        },
        Flatten(path: "user.orders.@") {
          Fetch(service: "grading") {
            {
              ... on Order {
                __typename
                sku
                id
              }
            } =>
            {
              ... on Order {
                grade
              }
            }
          },
        },
        Flatten(path: "user.orders.@") {
          Fetch(service: "catalog") {
            {
              ... on Order {
                __typename
                grade
                id
              }
            } =>
            {
              ... on Order {
                name
              }
            }
          },
        },
      },
    },
    "#);

    Ok(())
}

/// Related: https://github.com/graphql-hive/router/issues/1539
///
/// `@requires(fields: "sku weight")`, where `sku` comes from the subgraph that resolved the parent,
/// and `weight` from a third one.
/// Only `sku` can be added to the parent fetch.
/// `weight` still needs a fetch of its own.
#[test]
fn requires_from_two_subgraphs_after_entity_hop() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        {
          user {
            orders {
              name
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/requires-from-two-subgraphs.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "users") {
          {
            user {
              __typename
              id
            }
          }
        },
        Flatten(path: "user") {
          Fetch(service: "orders") {
            {
              ... on User {
                __typename
                id
              }
            } =>
            {
              ... on User {
                orders {
                  __typename
                  id
                  sku
                }
              }
            }
          },
        },
        Flatten(path: "user.orders.@") {
          Fetch(service: "warehouse") {
            {
              ... on Order {
                __typename
                id
              }
            } =>
            {
              ... on Order {
                weight
              }
            }
          },
        },
        Flatten(path: "user.orders.@") {
          Fetch(service: "catalog") {
            {
              ... on Order {
                __typename
                sku
                weight
                id
              }
            } =>
            {
              ... on Order {
                name
              }
            }
          },
        },
      },
    },
    "#);

    Ok(())
}

/// Related: https://github.com/graphql-hive/router/issues/1539
///
/// The `@requires` sits two entity hops down (`user -> cart -> orders`).
/// The required field should go into the fetch that resolved the last hop.
#[test]
fn requires_after_two_entity_hops() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        {
          user {
            cart {
              orders {
                name
              }
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/requires-after-two-hops.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          {
            user {
              __typename
              id
            }
          }
        },
        Flatten(path: "user") {
          Fetch(service: "b") {
            {
              ... on User {
                __typename
                id
              }
            } =>
            {
              ... on User {
                cart {
                  __typename
                  id
                }
              }
            }
          },
        },
        Flatten(path: "user.cart") {
          Fetch(service: "c") {
            {
              ... on Cart {
                __typename
                id
              }
            } =>
            {
              ... on Cart {
                orders {
                  __typename
                  id
                  sku
                }
              }
            }
          },
        },
        Flatten(path: "user.cart.orders.@") {
          Fetch(service: "catalog") {
            {
              ... on Order {
                __typename
                sku
                id
              }
            } =>
            {
              ... on Order {
                name
              }
            }
          },
        },
      },
    },
    "#);

    Ok(())
}

/// `pricing` has the keys `id` and `sku region`. `tax` there requires `price` from `catalog`.
/// The requirements are fetched from `products`, where `product` comes from, so they don't
/// wait for the `pricing` call. `products` only knows `id`, the key that call used, so that's
/// the key `pricing` is re-entered with. It used to pick `sku region`, and ask `products` for
/// fields it doesn't have.
#[test]
fn requires_reentry_uses_the_key_its_source_has() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        {
          product {
            tax
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/requires-reentry-parent-key.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "products") {
          {
            product {
              __typename
              id
            }
          }
        },
        Flatten(path: "product") {
          Fetch(service: "catalog") {
            {
              ... on Product {
                __typename
                id
              }
            } =>
            {
              ... on Product {
                price
              }
            }
          },
        },
        Flatten(path: "product") {
          Fetch(service: "pricing") {
            {
              ... on Product {
                __typename
                price
                id
              }
            } =>
            {
              ... on Product {
                tax
              }
            }
          },
        },
      },
    },
    "#);

    Ok(())
}
