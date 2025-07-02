use crate::{
    tests::testkit::{build_query_plan, init_logger},
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
    let query_plan = build_query_plan(
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
              ... on Product {
                __typename
                upc
              }
            } =>
            {
              ... on Product {
                _internal_qp_alias_0: price(withDiscount: false)
                price(withDiscount: true)
              }
            }
          },
        },
        Parallel {
          Flatten(path: "products.@") {
            Fetch(service: "inventory") {
                ... on Product {
                  __typename
                  price: _internal_qp_alias_0
                  upc
                }
              } =>
              {
                ... on Product {
                  reducedPrice
                }
              }
            },
          },
          Flatten(path: "products.@") {
            Fetch(service: "inventory") {
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
    },
    "#);
    insta::assert_snapshot!(format!("{}", serde_json::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Sequence",
        "nodes": [
          {
            "kind": "Fetch",
            "serviceName": "inventory",
            "operationKind": "query",
            "operation": "query{products{__typename upc}}"
          },
          {
            "kind": "Flatten",
            "path": [
              "products",
              "@"
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "products",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Product{_internal_qp_alias_0: price(withDiscount: false) price(withDiscount: true)}}}",
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
            "kind": "Parallel",
            "nodes": [
              {
                "kind": "Flatten",
                "path": [
                  "products",
                  "@"
                ],
                "node": {
                  "kind": "Fetch",
                  "serviceName": "inventory",
                  "operationKind": "query",
                  "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Product{reducedPrice}}}",
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
                }
              },
              {
                "kind": "Flatten",
                "path": [
                  "products",
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
    let query_plan = build_query_plan(
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
    insta::assert_snapshot!(format!("{}", serde_json::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Sequence",
        "nodes": [
          {
            "kind": "Fetch",
            "serviceName": "inventory",
            "operationKind": "query",
            "operation": "query{products{__typename upc}}"
          },
          {
            "kind": "Flatten",
            "path": [
              "products",
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
              "products",
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
    let query_plan = build_query_plan(
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
              price
              upc
            }
          }
        },
        Flatten(path: "products.@") {
          Fetch(service: "inventory") {
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
    insta::assert_snapshot!(format!("{}", serde_json::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Sequence",
        "nodes": [
          {
            "kind": "Fetch",
            "serviceName": "products",
            "operationKind": "query",
            "operation": "query{products{__typename price upc}}"
          },
          {
            "kind": "Flatten",
            "path": [
              "products",
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
    let query_plan = build_query_plan(
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
              price
              upc
            }
          }
        },
        Flatten(path: "products.@") {
          Fetch(service: "inventory") {
              ... on Product {
                __typename
                price
                upc
              }
            } =>
            {
              ... on Product {
                isAvailable
                isExpensive
              }
            }
          },
        },
      },
    },
    "#);
    insta::assert_snapshot!(format!("{}", serde_json::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Sequence",
        "nodes": [
          {
            "kind": "Fetch",
            "serviceName": "products",
            "operationKind": "query",
            "operation": "query{products{__typename price upc}}"
          },
          {
            "kind": "Flatten",
            "path": [
              "products",
              "@"
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "inventory",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Product{isAvailable isExpensive}}}",
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
    let query_plan =
        build_query_plan("fixture/tests/simple-requires.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "products") {
          {
            products {
              __typename
              price
              upc
              weight
            }
          }
        },
        Flatten(path: "products.@") {
          Fetch(service: "inventory") {
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
    insta::assert_snapshot!(format!("{}", serde_json::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Sequence",
        "nodes": [
          {
            "kind": "Fetch",
            "serviceName": "products",
            "operationKind": "query",
            "operation": "query{products{__typename price upc weight}}"
          },
          {
            "kind": "Flatten",
            "path": [
              "products",
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
    let query_plan = build_query_plan(
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
              price
              upc
              weight
            }
          }
        },
        Flatten(path: "products.@") {
          Fetch(service: "inventory") {
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
                shippingEstimate2
              }
            }
          },
        },
      },
    },
    "#);
    insta::assert_snapshot!(format!("{}", serde_json::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Sequence",
        "nodes": [
          {
            "kind": "Fetch",
            "serviceName": "products",
            "operationKind": "query",
            "operation": "query{products{__typename price upc weight}}"
          },
          {
            "kind": "Flatten",
            "path": [
              "products",
              "@"
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "inventory",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Product{shippingEstimate shippingEstimate2}}}",
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
    let query_plan = build_query_plan(
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
              price
              upc
              weight
            }
          }
        },
        Flatten(path: "products.@") {
          Fetch(service: "inventory") {
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
    insta::assert_snapshot!(format!("{}", serde_json::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Sequence",
        "nodes": [
          {
            "kind": "Fetch",
            "serviceName": "products",
            "operationKind": "query",
            "operation": "query{products{__typename price upc weight}}"
          },
          {
            "kind": "Flatten",
            "path": [
              "products",
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
    let query_plan = build_query_plan("fixture/tests/keys-mashup.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "b") {
          {
            b {
              a {
                __typename
                compositeId {
                  three
                  two
                }
                id
              }
              id
            }
          }
        },
        Flatten(path: "b.a.@") {
          Fetch(service: "a") {
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
    insta::assert_snapshot!(format!("{}", serde_json::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Sequence",
        "nodes": [
          {
            "kind": "Fetch",
            "serviceName": "b",
            "operationKind": "query",
            "operation": "query{b{a{__typename compositeId{three two} id} id}}"
          },
          {
            "kind": "Flatten",
            "path": [
              "b",
              "a",
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
              "b",
              "a",
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
    let query_plan = build_query_plan("fixture/tests/deep-requires.supergraph.graphql", document)?;

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
