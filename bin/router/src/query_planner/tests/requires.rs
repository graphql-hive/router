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

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] inventory       -
      [4] products        [2]
      [5] inventory       [4]

    [2] inventory
      out
        { products { __typename upc } }

    [4] products   after [2]
      merge
        products.@
      in
        ... on Product { __typename upc }
      out
        ... on Product { price(withDiscount: true) _internal_qp_alias_0: price(withDiscount: false) }

    [5] inventory   after [4]
      alias _e0
        merge
          products.@
        in
          ... on Product { __typename price: _internal_qp_alias_0 upc }
        out
          ... on Product { reducedPrice }
      alias _e1
        merge
          products.@
        in
          ... on Product { __typename price upc }
        out
          ... on Product { isExpensive }
    ");
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

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] inventory       -
      [4] products        [2]
      [3] inventory       [4]

    [2] inventory
      out
        { products { __typename upc } }

    [4] products   after [2]
      merge
        products.@
      in
        ... on Product { __typename upc }
      out
        ... on Product { price(withDiscount: true) }

    [3] inventory   after [4]
      merge
        products.@
      in
        ... on Product { __typename price upc }
      out
        ... on Product { isExpensive }
    ");
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

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] products        -
      [4] inventory       [2]

    [2] products
      out
        { products { __typename upc price } }

    [4] inventory   after [2]
      merge
        products.@
      in
        ... on Product { __typename price upc }
      out
        ... on Product { isExpensive }
    ");
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

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] products        -
      [4] inventory       [2]

    [2] products
      out
        { products { __typename upc price } }

    [4] inventory   after [2]
      merge
        products.@
      in
        ... on Product { __typename price upc }
      out
        ... on Product { isExpensive isAvailable }
    ");
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

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] products        -
      [4] inventory       [2]

    [2] products
      out
        { products { __typename upc price weight } }

    [4] inventory   after [2]
      merge
        products.@
      in
        ... on Product { __typename price weight upc }
      out
        ... on Product { shippingEstimate }
    ");
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

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] products        -
      [4] inventory       [2]

    [2] products
      out
        { products { __typename upc price weight } }

    [4] inventory   after [2]
      merge
        products.@
      in
        ... on Product { __typename price weight upc }
      out
        ... on Product { shippingEstimate shippingEstimate2 }
    ");
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
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/simple_requires_with_child.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] products        -
      [4] inventory       [2]

    [2] products
      out
        { products { __typename upc price weight } }

    [4] inventory   after [2]
      merge
        products.@
      in
        ... on Product { __typename price weight upc }
      out
        ... on Product { shippingEstimate { price } }
    ");
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

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] b               -
      [5] a               [2]
      [4] b               [5]

    [2] b
      out
        { b { id a { __typename id compositeId { two three } } } }

    [5] a   after [2]
      merge
        b.a.@
      in
        ... on A { __typename id }
      out
        ... on A { name }

    [4] b   after [5]
      merge
        b.a.@
      in
        ... on A { __typename name id compositeId { two three } }
      out
        ... on A { nameInB }
    ");
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

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] a               -
      [5] b               [2]
      [6] a               [5]
      [4] b               [6]

    [2] a
      out
        { feed { __typename id } }

    [5] b   after [2]
      merge
        feed.@
      in
        ... on Post { __typename id }
      out
        ... on Post { comments(limit: 3) { __typename id } }

    [6] a   after [5]
      merge
        feed.@.comments.@
      in
        ... on Comment { __typename id }
      out
        ... on Comment { authorId }

    [4] b   after [6]
      merge
        feed.@
      in
        ... on Post { __typename comments { authorId } id }
      out
        ... on Post { author { id } }
    ");

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
    fetches
      fetch               after
      [2] subgraph-a      -
      [5] subgraph-b      [2]
      [3] subgraph-a      [5]

    [2] subgraph-a
      out
        {
          productSearchResult(partialCriteria: "partialCriteria") {
            __typename
            searchCriteria { __typename partialCriteria }
          }
        }

    [5] subgraph-b   after [2]
      merge
        productSearchResult.searchCriteria
      in
        ... on ProductSearchCriteria { __typename partialCriteria }
      out
        ... on ProductSearchCriteria { fullCriteria }

    [3] subgraph-a   after [5]
      merge
        productSearchResult
      in
        ... on ProductSearchResultResponse {
          __typename
          searchCriteria { fullCriteria partialCriteria }
        }
      out
        ... on ProductSearchResultResponse { bestOffer { price } }
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

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] users           -
      [3] orders          [2]
      [7] grading         [3]
      [5] catalog         [7]

    [2] users
      out
        { user { __typename id } }

    [3] orders   after [2]
      merge
        user
      in
        ... on User { __typename id }
      out
        ... on User { orders { __typename id sku } }

    [7] grading   after [3]
      merge
        user.orders.@
      in
        ... on Order { __typename sku id }
      out
        ... on Order { grade }

    [5] catalog   after [7]
      merge
        user.orders.@
      in
        ... on Order { __typename grade id }
      out
        ... on Order { name }
    ");

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

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] users           -
      [3] orders          [2]
      [7] warehouse       [3]
      [5] catalog         [7]

    [2] users
      out
        { user { __typename id } }

    [3] orders   after [2]
      merge
        user
      in
        ... on User { __typename id }
      out
        ... on User { orders { __typename id sku } }

    [7] warehouse   after [3]
      merge
        user.orders.@
      in
        ... on Order { __typename id }
      out
        ... on Order { weight }

    [5] catalog   after [7]
      merge
        user.orders.@
      in
        ... on Order { __typename sku weight id }
      out
        ... on Order { name }
    ");

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

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] a               -
      [3] b               [2]
      [4] c               [3]
      [6] catalog         [4]

    [2] a
      out
        { user { __typename id } }

    [3] b   after [2]
      merge
        user
      in
        ... on User { __typename id }
      out
        ... on User { cart { __typename id } }

    [4] c   after [3]
      merge
        user.cart
      in
        ... on Cart { __typename id }
      out
        ... on Cart { orders { __typename id sku } }

    [6] catalog   after [4]
      merge
        user.cart.orders.@
      in
        ... on Order { __typename sku id }
      out
        ... on Order { name }
    ");

    Ok(())
}

// `shippingEstimate` in `shipping` requires `weight` from `inventory`, so `shipping` gets called
// again for it with its key `id sku`. The product came from `products`, which has no `sku`, it
// only came in through `catalog` on the way to `shipping`. So the key comes from the call to
// `shipping`, not from `products`, which can't resolve it.
#[test]
fn requires_reenters_with_a_key_its_source_lacks() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation("{ product { shippingEstimate } }");
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/requires-reentry-key-from-elsewhere.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] products        -
      [4] catalog         [2]
      [7] inventory       [4]
      [3] shipping        [7]

    [2] products
      out
        { product { __typename id } }

    [4] catalog   after [2]
      merge
        product
      in
        ... on Product { __typename id }
      out
        ... on Product { sku }

    [7] inventory   after [4]
      merge
        product
      in
        ... on Product { __typename sku }
      out
        ... on Product { weight }

    [3] shipping   after [7]
      merge
        product
      in
        ... on Product { __typename id sku weight }
      out
        ... on Product { shippingEstimate }
    ");

    Ok(())
}

// The same, but `shipping` is also called for `carrier`, so that call stays its own step and the
// key is added to its output. `inventory` then waits for `shipping` too, though `sku` from
// `catalog` is enough for it.
#[test]
fn requires_reenters_with_a_key_its_source_lacks_from_a_call_of_its_own(
) -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation("{ product { carrier shippingEstimate } }");
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/requires-reentry-key-from-elsewhere.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] products        -
      [4] catalog         [2]
      [7] inventory       [4]
      [3] shipping        [7]

    [2] products
      out
        { product { __typename id } }

    [4] catalog   after [2]
      merge
        product
      in
        ... on Product { __typename id }
      out
        ... on Product { sku }

    [7] inventory   after [4]
      merge
        product
      in
        ... on Product { __typename sku }
      out
        ... on Product { weight }

    [3] shipping   after [7]
      merge
        product
      in
        ... on Product { __typename id sku weight }
      out
        ... on Product { carrier shippingEstimate }
    ");

    Ok(())
}

// `shipping` has two keys, `id sku` and `upc`. `products` can't resolve the first one, it has no
// `sku`, but it has `upc`. So `shipping` gets called again with `upc` read from `products`.
#[test]
fn requires_reenters_with_a_key_its_source_has() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation("{ product { shippingEstimate } }");
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/requires-reentry-key-two-keys.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] products        -
      [5] inventory       [2]
      [4] shipping        [5]

    [2] products
      out
        { product { __typename upc } }

    [5] inventory   after [2]
      merge
        product
      in
        ... on Product { __typename upc }
      out
        ... on Product { weight }

    [4] shipping   after [5]
      merge
        product
      in
        ... on Product { __typename weight upc }
      out
        ... on Product { shippingEstimate }
    ");

    Ok(())
}

// `shipping` is keyed by `id sku region`, and no other subgraph has all three: `sku` comes from
// `catalog` and `region` from `regions`. They're all in the response before the call to
// `shipping` though, so `shipping` gets called once, after `inventory`, with what's there.
#[test]
fn requires_reenters_with_a_key_from_more_than_one_subgraph() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation("{ product { carrier shippingEstimate } }");
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/requires-reentry-key-split.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] products        -
      [5] regions         [2]
      [4] catalog         [2]
      [8] inventory       [5], [4]
      [3] shipping        [8]

    [2] products
      out
        { product { __typename id } }

    [5] regions   after [2]
      merge
        product
      in
        ... on Product { __typename id }
      out
        ... on Product { region }

    [4] catalog   after [2]
      merge
        product
      in
        ... on Product { __typename id }
      out
        ... on Product { sku }

    [8] inventory   after [5], [4]
      merge
        product
      in
        ... on Product { __typename sku }
      out
        ... on Product { weight }

    [3] shipping   after [8]
      merge
        product
      in
        ... on Product { __typename id sku region weight }
      out
        ... on Product { carrier shippingEstimate }
    ");

    Ok(())
}

// Like `requires_reenters_with_a_key_its_source_lacks_from_a_call_of_its_own`, with `carrier`
// under `@include`. It still goes to the one call to `shipping`, under its condition.
#[test]
fn requires_reenters_with_a_key_its_source_lacks_next_to_a_conditional_field(
) -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        "query ($a: Boolean!) { product { carrier @include(if: $a) shippingEstimate } }",
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/requires-reentry-key-from-elsewhere.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    variables
      $a: Boolean!

    fetches
      fetch               after
      [2] products        -
      [6] catalog         [2]
      [9] inventory       [6]
      [3] shipping        [9]

    [2] products
      out
        { product { __typename id } }

    [6] catalog   after [2]
      merge
        product
      in
        ... on Product { __typename id }
      out
        ... on Product { sku ... on Product @include(if: $a) { sku } }
      vars
        $a

    [9] inventory   after [6]
      merge
        product
      in
        ... on Product { __typename sku }
      out
        ... on Product { weight }

    [3] shipping   after [9]
      merge
        product
      in
        ... on Product { __typename id sku weight }
      out
        ... on Product { ... on Product @include(if: $a) { carrier } shippingEstimate }
      vars
        $a
    ");

    Ok(())
}

#[test]
fn leaf_skips_requires_when_another_subgraph_has_it_plain() -> Result<(), Box<dyn Error>> {
    init_logger();
    // `a` has `shipping` but needs `weight` from `b` for it, `c` has it with nothing needed:
    // going to `c` is one entity call instead of two
    let document = parse_operation(
        r#"
        query {
          products {
            shipping
          }
        }"#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/requires-or-plain.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] a               -
      [3] c               [2]

    [2] a
      out
        { products { __typename id } }

    [3] c   after [2]
      merge
        products.@
      in
        ... on Product { __typename id }
      out
        ... on Product { shipping }
    ");
    Ok(())
}
