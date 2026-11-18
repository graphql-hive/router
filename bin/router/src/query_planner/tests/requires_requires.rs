use crate::query_planner::{
    tests::testkit::{build_query_plan_with_defaults, init_logger},
    utils::parsing::parse_operation,
};
use std::error::Error;

/// Promoted from generated seed 4. Overlapping `@requires` chains selected through repeated
/// product fields and fragments must survive fetch-step merges without stale step references.
#[test]
fn overlapping_requires_chains_survive_fetch_step_merges() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(include_str!("fixtures/overlapping_requires_chains.graphql"));
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/requires_requires.supergraph.graphql",
        document,
    )?;
    insta::assert_snapshot!(format!("{query_plan}"), @r"
    fetches
      fetch               after
      [2] b               -
      [7] c               [2]
      [36] a              [2]
      [10] c              [36]
      [22] d              [7], [10]

    [2] b
      out
        { product { __typename id hasDiscount } }

    [7] c   after [2]
      merge
        product
      in
        ... on Product { __typename hasDiscount id }
      out
        ... on Product { isExpensiveWithDiscount }

    [36] a   after [2]
      merge
        product
      in
        ... on Product { __typename id }
      out
        ... on Product { price }

    [10] c   after [36]
      merge
        product
      in
        ... on Product { __typename price id }
      out
        ... on Product { isExpensive }

    [22] d   after [7], [10]
      merge
        product
      in
        ... on Product { __typename isExpensive id isExpensiveWithDiscount }
      out
        ... on Product {
          canAfford
          requestedB: canAffordWithAndWithoutDiscount
          canAffordWithAndWithoutDiscount
          requestedA: canAffordWithAndWithoutDiscount
        }
    ");

    Ok(())
}

#[test]
fn one() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          product {
            canAffordWithDiscount
          }
        }"#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/requires_requires.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] b               -
      [7] c               [2]
      [4] d               [7]

    [2] b
      out
        { product { __typename id hasDiscount } }

    [7] c   after [2]
      merge
        product
      in
        ... on Product { __typename hasDiscount id }
      out
        ... on Product { isExpensiveWithDiscount }

    [4] d   after [7]
      merge
        product
      in
        ... on Product { __typename isExpensiveWithDiscount id }
      out
        ... on Product { canAffordWithDiscount }
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
            "operation": "{product{__typename id hasDiscount}}"
          },
          {
            "kind": "Flatten",
            "path": [
              {
                "Field": "product"
              }
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "c",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Product{isExpensiveWithDiscount}}}",
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
                      "name": "hasDiscount"
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
                "Field": "product"
              }
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "d",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Product{canAffordWithDiscount}}}",
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
                      "name": "isExpensiveWithDiscount"
                    },
                    {
                      "kind": "Field",
                      "name": "id"
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
fn one_with_one_local() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          product {
            fieldInD
            canAffordWithDiscount
          }
        }"#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/requires_requires.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] b               -
      [7] c               [2]
      [3] d               [7]

    [2] b
      out
        { product { __typename id hasDiscount } }

    [7] c   after [2]
      merge
        product
      in
        ... on Product { __typename hasDiscount id }
      out
        ... on Product { isExpensiveWithDiscount }

    [3] d   after [7]
      merge
        product
      in
        ... on Product { __typename id isExpensiveWithDiscount }
      out
        ... on Product { fieldInD canAffordWithDiscount }
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
            "operation": "{product{__typename id hasDiscount}}"
          },
          {
            "kind": "Flatten",
            "path": [
              {
                "Field": "product"
              }
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "c",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Product{isExpensiveWithDiscount}}}",
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
                      "name": "hasDiscount"
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
                "Field": "product"
              }
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "d",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Product{fieldInD canAffordWithDiscount}}}",
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
                      "name": "id"
                    },
                    {
                      "kind": "Field",
                      "name": "isExpensiveWithDiscount"
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
fn two_fields_with_the_same_requirements() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          product {
            canAffordWithDiscount
            canAffordWithDiscount2
          }
        }"#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/requires_requires.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] b               -
      [7] c               [2]
      [10] d              [7]

    [2] b
      out
        { product { __typename id hasDiscount } }

    [7] c   after [2]
      merge
        product
      in
        ... on Product { __typename hasDiscount id }
      out
        ... on Product { isExpensiveWithDiscount }

    [10] d   after [7]
      merge
        product
      in
        ... on Product { __typename isExpensiveWithDiscount id }
      out
        ... on Product { canAffordWithDiscount2 canAffordWithDiscount }
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
            "operation": "{product{__typename id hasDiscount}}"
          },
          {
            "kind": "Flatten",
            "path": [
              {
                "Field": "product"
              }
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "c",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Product{isExpensiveWithDiscount}}}",
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
                      "name": "hasDiscount"
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
                "Field": "product"
              }
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "d",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Product{canAffordWithDiscount2 canAffordWithDiscount}}}",
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
                      "name": "isExpensiveWithDiscount"
                    },
                    {
                      "kind": "Field",
                      "name": "id"
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
fn one_more() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          product {
            canAfford
          }
        }"#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/requires_requires.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] b               -
      [9] a               [2]
      [7] c               [9]
      [4] d               [7]

    [2] b
      out
        { product { __typename id } }

    [9] a   after [2]
      merge
        product
      in
        ... on Product { __typename id }
      out
        ... on Product { price }

    [7] c   after [9]
      merge
        product
      in
        ... on Product { __typename price id }
      out
        ... on Product { isExpensive }

    [4] d   after [7]
      merge
        product
      in
        ... on Product { __typename isExpensive id }
      out
        ... on Product { canAfford }
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
            "operation": "{product{__typename id}}"
          },
          {
            "kind": "Flatten",
            "path": [
              {
                "Field": "product"
              }
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "a",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Product{price}}}",
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
                "Field": "product"
              }
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "c",
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
                "Field": "product"
              }
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "d",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Product{canAfford}}}",
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
                      "name": "isExpensive"
                    },
                    {
                      "kind": "Field",
                      "name": "id"
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
fn another_two_fields_with_the_same_requirements() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          product {
            canAfford
            canAfford2
          }
        }"#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/requires_requires.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] b               -
      [15] a              [2]
      [7] c               [15]
      [10] d              [7]

    [2] b
      out
        { product { __typename id } }

    [15] a   after [2]
      merge
        product
      in
        ... on Product { __typename id }
      out
        ... on Product { price }

    [7] c   after [15]
      merge
        product
      in
        ... on Product { __typename price id }
      out
        ... on Product { isExpensive }

    [10] d   after [7]
      merge
        product
      in
        ... on Product { __typename isExpensive id }
      out
        ... on Product { canAfford2 canAfford }
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
            "operation": "{product{__typename id}}"
          },
          {
            "kind": "Flatten",
            "path": [
              {
                "Field": "product"
              }
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "a",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Product{price}}}",
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
                "Field": "product"
              }
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "c",
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
                "Field": "product"
              }
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "d",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Product{canAfford2 canAfford}}}",
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
                      "name": "isExpensive"
                    },
                    {
                      "kind": "Field",
                      "name": "id"
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
fn two_fields() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          product {
            canAffordWithDiscount
            canAfford
          }
        }"#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/requires_requires.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] b               -
      [7] c               [2]
      [15] a              [2]
      [13] c              [15]
      [4] d               [7], [13]

    [2] b
      out
        { product { __typename id hasDiscount } }

    [7] c   after [2]
      merge
        product
      in
        ... on Product { __typename hasDiscount id }
      out
        ... on Product { isExpensiveWithDiscount }

    [15] a   after [2]
      merge
        product
      in
        ... on Product { __typename id }
      out
        ... on Product { price }

    [13] c   after [15]
      merge
        product
      in
        ... on Product { __typename price id }
      out
        ... on Product { isExpensive }

    [4] d   after [7], [13]
      merge
        product
      in
        ... on Product { __typename isExpensiveWithDiscount id isExpensive }
      out
        ... on Product { canAffordWithDiscount canAfford }
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
            "operation": "{product{__typename id hasDiscount}}"
          },
          {
            "kind": "Parallel",
            "nodes": [
              {
                "kind": "Flatten",
                "path": [
                  {
                    "Field": "product"
                  }
                ],
                "node": {
                  "kind": "Fetch",
                  "serviceName": "c",
                  "operationKind": "query",
                  "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Product{isExpensiveWithDiscount}}}",
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
                          "name": "hasDiscount"
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
                    "Field": "product"
                  }
                ],
                "node": {
                  "kind": "Fetch",
                  "serviceName": "a",
                  "operationKind": "query",
                  "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Product{price}}}",
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
                          "name": "id"
                        }
                      ]
                    }
                  ]
                }
              }
            ]
          },
          {
            "kind": "Flatten",
            "path": [
              {
                "Field": "product"
              }
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "c",
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
                "Field": "product"
              }
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "d",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Product{canAffordWithDiscount canAfford}}}",
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
                      "name": "isExpensiveWithDiscount"
                    },
                    {
                      "kind": "Field",
                      "name": "id"
                    },
                    {
                      "kind": "Field",
                      "name": "isExpensive"
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
fn two_fields_same_requirement_different_order() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          product {
            canAffordWithAndWithoutDiscount
            canAffordWithAndWithoutDiscount2
          }
        }"#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/requires_requires.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] b               -
      [7] c               [2]
      [18] a              [2]
      [10] c              [18]
      [13] d              [7], [10]

    [2] b
      out
        { product { __typename id hasDiscount } }

    [7] c   after [2]
      merge
        product
      in
        ... on Product { __typename hasDiscount id }
      out
        ... on Product { isExpensiveWithDiscount }

    [18] a   after [2]
      merge
        product
      in
        ... on Product { __typename id }
      out
        ... on Product { price }

    [10] c   after [18]
      merge
        product
      in
        ... on Product { __typename price id }
      out
        ... on Product { isExpensive }

    [13] d   after [7], [10]
      merge
        product
      in
        ... on Product { __typename isExpensive isExpensiveWithDiscount id }
      out
        ... on Product { canAffordWithAndWithoutDiscount2 canAffordWithAndWithoutDiscount }
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
            "operation": "{product{__typename id hasDiscount}}"
          },
          {
            "kind": "Parallel",
            "nodes": [
              {
                "kind": "Flatten",
                "path": [
                  {
                    "Field": "product"
                  }
                ],
                "node": {
                  "kind": "Fetch",
                  "serviceName": "c",
                  "operationKind": "query",
                  "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Product{isExpensiveWithDiscount}}}",
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
                          "name": "hasDiscount"
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
                    "Field": "product"
                  }
                ],
                "node": {
                  "kind": "Fetch",
                  "serviceName": "a",
                  "operationKind": "query",
                  "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Product{price}}}",
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
                          "name": "id"
                        }
                      ]
                    }
                  ]
                }
              }
            ]
          },
          {
            "kind": "Flatten",
            "path": [
              {
                "Field": "product"
              }
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "c",
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
                "Field": "product"
              }
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "d",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Product{canAffordWithAndWithoutDiscount2 canAffordWithAndWithoutDiscount}}}",
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
                      "name": "isExpensive"
                    },
                    {
                      "kind": "Field",
                      "name": "isExpensiveWithDiscount"
                    },
                    {
                      "kind": "Field",
                      "name": "id"
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
fn many() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          product {
            id
            price
            hasDiscount
            isExpensive
            isExpensiveWithDiscount
            canAfford
            canAfford2
            canAffordWithDiscount
            canAffordWithDiscount2
          }
        }"#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/requires_requires.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] b               -
      [8] c               [2]
      [23] a              [2]
      [5] c               [23]
      [18] d              [8], [5]

    [2] b
      out
        { product { __typename id hasDiscount } }

    [8] c   after [2]
      merge
        product
      in
        ... on Product { __typename hasDiscount id }
      out
        ... on Product { isExpensiveWithDiscount }

    [23] a   after [2]
      merge
        product
      in
        ... on Product { __typename id }
      out
        ... on Product { price }

    [5] c   after [23]
      merge
        product
      in
        ... on Product { __typename price id }
      out
        ... on Product { isExpensive }

    [18] d   after [8], [5]
      merge
        product
      in
        ... on Product { __typename isExpensive id isExpensiveWithDiscount }
      out
        ... on Product { canAfford2 canAfford canAffordWithDiscount2 canAffordWithDiscount }
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
            "operation": "{product{__typename id hasDiscount}}"
          },
          {
            "kind": "Parallel",
            "nodes": [
              {
                "kind": "Flatten",
                "path": [
                  {
                    "Field": "product"
                  }
                ],
                "node": {
                  "kind": "Fetch",
                  "serviceName": "c",
                  "operationKind": "query",
                  "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Product{isExpensiveWithDiscount}}}",
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
                          "name": "hasDiscount"
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
                    "Field": "product"
                  }
                ],
                "node": {
                  "kind": "Fetch",
                  "serviceName": "a",
                  "operationKind": "query",
                  "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Product{price}}}",
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
                          "name": "id"
                        }
                      ]
                    }
                  ]
                }
              }
            ]
          },
          {
            "kind": "Flatten",
            "path": [
              {
                "Field": "product"
              }
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "c",
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
                "Field": "product"
              }
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "d",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Product{canAfford2 canAfford canAffordWithDiscount2 canAffordWithDiscount}}}",
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
                      "name": "isExpensive"
                    },
                    {
                      "kind": "Field",
                      "name": "id"
                    },
                    {
                      "kind": "Field",
                      "name": "isExpensiveWithDiscount"
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
