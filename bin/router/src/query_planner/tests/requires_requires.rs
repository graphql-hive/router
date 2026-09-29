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
    insta::assert_snapshot!(format!("{query_plan}"), @r###"
    QueryPlan {
      Sequence {
        Fetch(service: "b") {
          query ($v1:Boolean!,$v10:Boolean!,$v11:Boolean!,$v13:Boolean!,$v15:Boolean!=true,$v16:Boolean!,$v17:Boolean!,$v18:Boolean!,$v19:Boolean!,$v2:Boolean!,$v21:Boolean!=false,$v22:Boolean!=false,$v23:Boolean!=true,$v24:Boolean!,$v25:Boolean!=true,$v26:Boolean!,$v27:Boolean!,$v28:Boolean!,$v29:Boolean!,$v3:Boolean!=true,$v4:Boolean!,$v5:Boolean!,$v6:Boolean!,$v7:Boolean!,$v8:Boolean!,$v9:Boolean!) {
            product {
              __typename
              id
              ... on Product @include(if: $v2) {
                __typename
                id
              }
              ... on Product @skip(if: $v5) {
                __typename
                hasDiscount @skip(if: $v3) @include(if: $v4)
              }
              ... on Product @include(if: $v17) {
                __typename
                id
                ... on Product @include(if: $v16) {
                  __typename
                  ... on Product @include(if: $v15) {
                    __typename
                    id
                    a6_id: id @include(if: $v13)
                  }
                  id
                  hasDiscount
                }
                ... on Product @skip(if: $v11) {
                  ...a
                }
                ... on Product @skip(if: $v6) @include(if: $v7) {
                  ...a
                }
                ... on Product @skip(if: $v10) {
                  ...a
                }
                ... on Product @skip(if: $v9) @include(if: $v8) {
                  ...a
                }
              }
              a3_hasDiscount: hasDiscount
              a20_hasDiscount: hasDiscount
              a2_id: id
              hasDiscount
              ... on Product @include(if: $v29) {
                ...a
              }
              ... on Product @include(if: $v1) {
                ...a
              }
            }
            __typename
            a16_product: product {
              ...a
            }
            ... on Query @skip(if: $v28) {
              __typename
              product {
                __typename
                id
                id @skip(if: $v19)
                a10_id: id
                ... on Product @skip(if: $v26) @include(if: $v27) {
                  __typename
                  id
                  ... on Product @skip(if: $v22) {
                    ...a
                  }
                  ... on Product @skip(if: $v23) @include(if: $v24) {
                    ...a
                  }
                  hasDiscount
                  ... on Product @skip(if: $v25) {
                    ...a
                  }
                }
                ... on Product @skip(if: $v28) {
                  ...a
                }
                ... on Product @include(if: $v18) {
                  ...a
                }
                ... on Product @include(if: $v21) {
                  ...a
                }
              }
            }
          }
          fragment a on Product {
            hasDiscount
            __typename
            id
          }
        },
        Parallel {
          BatchFetch(service: "c") {
            {
              _e0 {
                paths: [
                  "|[Query].product|[Product]"
                ]
                {
                  ... on Product {
                    __typename
                    hasDiscount
                    id
                  }
                }
              }
              _e1 {
                paths: [
                  "|[Query].product"
                ]
                {
                  ... on Product {
                    __typename
                    hasDiscount
                    id
                  }
                }
              }
              _e2 {
                paths: [
                  "product"
                ]
                {
                  ... on Product {
                    __typename
                    hasDiscount
                    id
                  }
                }
              }
              _e3 {
                paths: [
                  "product|[Product]"
                ]
                {
                  ... on Product {
                    __typename
                    hasDiscount
                    id
                  }
                }
              }
              _e4 {
                paths: [
                  "a16_product"
                ]
                {
                  ... on Product {
                    __typename
                    hasDiscount
                    id
                  }
                }
              }
            }
            ($v22:Boolean!=false,$v23:Boolean!=true,$v24:Boolean!,$v25:Boolean!=true,$v26:Boolean!,$v27:Boolean!,$v18:Boolean!,$v21:Boolean!=false,$v1:Boolean!,$v29:Boolean!,$v10:Boolean!,$v11:Boolean!,$v6:Boolean!,$v7:Boolean!,$v8:Boolean!,$v9:Boolean!) {
              _e0: _entities(representations: $__batch_reps_0) {
                ... on Product {
                  ... on Product @skip(if: $v25) {
                    ...a
                  }
                  ... on Product @skip(if: $v26) @include(if: $v27) {
                    a14_isExpensiveWithDiscount: isExpensiveWithDiscount
                  }
                  ... on Product @skip(if: $v23) @include(if: $v24) {
                    ...a
                  }
                  ... on Product @skip(if: $v22) {
                    ...a
                  }
                }
              }
              _e1: _entities(representations: $__batch_reps_1) {
                ... on Product {
                  ... on Product @include(if: $v21) {
                    ...a
                  }
                  ... on Product @include(if: $v18) {
                    ...a
                  }
                }
              }
              _e2: _entities(representations: $__batch_reps_2) {
                ... on Product {
                  a19_isExpensiveWithDiscount: isExpensiveWithDiscount
                  isExpensiveWithDiscount
                  ... on Product @include(if: $v1) {
                    ...a
                  }
                  ... on Product @include(if: $v29) {
                    ...a
                  }
                }
              }
              _e3: _entities(representations: $__batch_reps_3) {
                ... on Product {
                  ... on Product @skip(if: $v9) @include(if: $v8) {
                    ...a
                  }
                  ... on Product @skip(if: $v10) {
                    ...a
                  }
                  ... on Product @skip(if: $v6) @include(if: $v7) {
                    ...a
                  }
                  ... on Product @skip(if: $v11) {
                    ...a
                  }
                }
              }
              _e4: _entities(representations: $__batch_reps_4) {
                ... on Product {
                  ...b
                }
              }
            }
            fragment a on Product {
              ... on Product {
                ...b
              }
            }
            fragment b on Product {
              isExpensiveWithDiscount
            }
          },
          Include(if: $v16) {
            Flatten(path: "product|[Product]|[Product]") {
              Fetch(service: "c") {
                {
                  ... on Product {
                    __typename
                    hasDiscount
                    id
                  }
                } =>
                {
                  ... on Product {
                    isExpensiveWithDiscount
                  }
                }
              },
            },
          },
          Skip(if: $v28) {
            Flatten(path: "|[Query].product") {
              Fetch(service: "c") {
                {
                  ... on Product {
                    __typename
                    hasDiscount
                    id
                  }
                } =>
                {
                  ... on Product {
                    isExpensiveWithDiscount
                  }
                }
              },
            },
          },
          Skip(if: $v28) {
            Flatten(path: "|[Query].product") {
              Fetch(service: "a") {
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
          },
          BatchFetch(service: "a") {
            {
              _e0 {
                paths: [
                  "|[Query].product|[Product]"
                ]
                {
                  ... on Product {
                    __typename
                    id
                  }
                }
              }
              _e1 {
                paths: [
                  "product"
                ]
                {
                  ... on Product {
                    __typename
                    id
                  }
                }
              }
            }
            ($v22:Boolean!=false,$v23:Boolean!=true,$v24:Boolean!,$v29:Boolean!) {
              _e0: _entities(representations: $__batch_reps_0) {
                ... on Product {
                  ... on Product @skip(if: $v22) {
                    ...a
                  }
                  ... on Product @skip(if: $v23) @include(if: $v24) {
                    ...a
                  }
                }
              }
              _e1: _entities(representations: $__batch_reps_1) {
                ... on Product {
                  price
                  ... on Product @include(if: $v29) {
                    ...b
                  }
                }
              }
            }
            fragment a on Product {
              ... on Product {
                ...b
              }
            }
            fragment b on Product {
              price
            }
          },
          Skip(if: $v6) {
            Include(if: $v7) {
              Flatten(path: "product|[Product]") {
                Fetch(service: "a") {
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
            },
          },
          Include(if: $v15) {
            Flatten(path: "product|[Product]|[Product]|[Product]") {
              Fetch(service: "a") {
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
          },
          Include(if: $v14) {
            Flatten(path: "product|[Product]|[Product]|[Product]") {
              Fetch(service: "a") {
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
          },
          Include(if: $v2) {
            Flatten(path: "product|[Product]") {
              Fetch(service: "a") {
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
          },
          Skip(if: $v26) {
            Include(if: $v27) {
              Flatten(path: "|[Query].product|[Product]") {
                Fetch(service: "d") {
                  {
                    ... on Product {
                      __typename
                      id
                    }
                  } =>
                  {
                    ... on Product {
                      fieldInD
                    }
                  }
                },
              },
            },
          },
          Skip(if: $v12) {
            Flatten(path: "product|[Product]") {
              Fetch(service: "d") {
                {
                  ... on Product {
                    __typename
                    id
                  }
                } =>
                {
                  ... on Product {
                    a5_fieldInD: fieldInD
                  }
                }
              },
            },
          },
        },
        Parallel {
          Include(if: $v16) {
            Flatten(path: "product|[Product]|[Product]") {
              Fetch(service: "d") {
                {
                  ... on Product {
                    __typename
                    isExpensiveWithDiscount
                    id
                  }
                } =>
                {
                  ... on Product {
                    canAffordWithDiscount2
                  }
                }
              },
            },
          },
          Flatten(path: "a16_product") {
            Fetch(service: "d") {
              {
                ... on Product {
                  __typename
                  isExpensiveWithDiscount
                  id
                }
              } =>
              {
                ... on Product {
                  a18_canAffordWithDiscount: canAffordWithDiscount
                  a17_canAffordWithDiscount2: canAffordWithDiscount2
                  canAffordWithDiscount
                }
              }
            },
          },
          Skip(if: $v28) {
            Flatten(path: "|[Query].product") {
              Fetch(service: "c") {
                {
                  ... on Product {
                    __typename
                    price
                    id
                  }
                } =>
                {
                  ... on Product {
                    a12_isExpensive: isExpensive
                    a11_isExpensive: isExpensive
                    isExpensive
                  }
                }
              },
            },
          },
          BatchFetch(service: "c") {
            {
              _e0 {
                paths: [
                  "|[Query].product|[Product]"
                ]
                {
                  ... on Product {
                    __typename
                    price
                    id
                  }
                }
              }
              _e1 {
                paths: [
                  "product"
                ]
                {
                  ... on Product {
                    __typename
                    price
                    id
                  }
                }
              }
            }
            ($v22:Boolean!=false,$v23:Boolean!=true,$v24:Boolean!,$v29:Boolean!) {
              _e0: _entities(representations: $__batch_reps_0) {
                ... on Product {
                  ... on Product @skip(if: $v23) @include(if: $v24) {
                    ...a
                  }
                  ... on Product @skip(if: $v22) {
                    ...a
                  }
                }
              }
              _e1: _entities(representations: $__batch_reps_1) {
                ... on Product {
                  ... on Product @include(if: $v29) {
                    ...b
                  }
                  isExpensive
                }
              }
            }
            fragment a on Product {
              ... on Product {
                ...b
              }
            }
            fragment b on Product {
              isExpensive
            }
          },
          Skip(if: $v6) {
            Include(if: $v7) {
              Flatten(path: "product|[Product]") {
                Fetch(service: "c") {
                  {
                    ... on Product {
                      __typename
                      price
                      id
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
          Include(if: $v15) {
            Flatten(path: "product|[Product]|[Product]|[Product]") {
              Fetch(service: "c") {
                {
                  ... on Product {
                    __typename
                    price
                    id
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
          Include(if: $v14) {
            Flatten(path: "product|[Product]|[Product]|[Product]") {
              Fetch(service: "c") {
                {
                  ... on Product {
                    __typename
                    price
                    id
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
          Include(if: $v2) {
            Flatten(path: "product|[Product]") {
              Fetch(service: "c") {
                {
                  ... on Product {
                    __typename
                    price
                    id
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
        Parallel {
          Skip(if: $v28) {
            Flatten(path: "|[Query].product") {
              Fetch(service: "d") {
                {
                  ... on Product {
                    __typename
                    id
                    isExpensive
                    isExpensiveWithDiscount
                  }
                } =>
                {
                  ... on Product {
                    fieldInD
                    a8_fieldInD: fieldInD
                    canAfford2
                    a13_canAfford2: canAfford2
                    a9_canAfford2: canAfford2
                    canAffordWithAndWithoutDiscount2
                  }
                }
              },
            },
          },
          BatchFetch(service: "d") {
            {
              _e0 {
                paths: [
                  "|[Query].product|[Product]"
                ]
                {
                  ... on Product {
                    __typename
                    isExpensiveWithDiscount
                    isExpensive
                    id
                  }
                }
              }
              _e1 {
                paths: [
                  "product"
                ]
                {
                  ... on Product {
                    __typename
                    id
                    isExpensive
                    isExpensiveWithDiscount
                  }
                }
              }
              _e2 {
                paths: [
                  "product|[Product]"
                ]
                {
                  ... on Product {
                    __typename
                    isExpensiveWithDiscount
                    id
                    isExpensive
                  }
                }
              }
            }
            ($v22:Boolean!=false,$v23:Boolean!=true,$v24:Boolean!,$v10:Boolean!,$v11:Boolean!,$v6:Boolean!,$v7:Boolean!,$v8:Boolean!,$v9:Boolean!) {
              _e0: _entities(representations: $__batch_reps_0) {
                ... on Product {
                  ... on Product @skip(if: $v22) {
                    ...a
                  }
                  ... on Product @skip(if: $v23) @include(if: $v24) {
                    ...a
                  }
                }
              }
              _e1: _entities(representations: $__batch_reps_1) {
                ... on Product {
                  fieldInD
                  canAfford
                  a22_canAffordWithAndWithoutDiscount2: canAffordWithAndWithoutDiscount2
                  a24_canAffordWithDiscount: canAffordWithDiscount
                  a23_canAffordWithAndWithoutDiscount2: canAffordWithAndWithoutDiscount2
                  a21_canAffordWithAndWithoutDiscount: canAffordWithAndWithoutDiscount
                  canAffordWithDiscount2
                  canAffordWithDiscount
                  canAffordWithAndWithoutDiscount
                  a1_canAffordWithAndWithoutDiscount: canAffordWithAndWithoutDiscount
                }
              }
              _e2: _entities(representations: $__batch_reps_2) {
                ... on Product {
                  ... on Product @skip(if: $v10) {
                    canAffordWithDiscount
                  }
                  ... on Product @skip(if: $v9) @include(if: $v8) {
                    canAffordWithDiscount2
                  }
                  ... on Product @skip(if: $v11) {
                    a4_canAffordWithDiscount: canAffordWithDiscount
                  }
                  ... on Product @skip(if: $v6) @include(if: $v7) {
                    ...b
                  }
                }
              }
            }
            fragment a on Product {
              ... on Product {
                ...b
              }
            }
            fragment b on Product {
              canAffordWithAndWithoutDiscount
            }
          },
          Include(if: $v29) {
            Flatten(path: "product") {
              Fetch(service: "d") {
                {
                  ... on Product {
                    __typename
                    isExpensive
                    isExpensiveWithDiscount
                    id
                  }
                } =>
                {
                  ... on Product {
                    canAffordWithAndWithoutDiscount2
                  }
                }
              },
            },
          },
          Include(if: $v15) {
            Flatten(path: "product|[Product]|[Product]|[Product]") {
              Fetch(service: "d") {
                {
                  ... on Product {
                    __typename
                    isExpensive
                    id
                  }
                } =>
                {
                  ... on Product {
                    canAfford2
                  }
                }
              },
            },
          },
          Include(if: $v14) {
            Flatten(path: "product|[Product]|[Product]|[Product]") {
              Fetch(service: "d") {
                {
                  ... on Product {
                    __typename
                    isExpensive
                    id
                  }
                } =>
                {
                  ... on Product {
                    canAfford2
                  }
                }
              },
            },
          },
          Include(if: $v2) {
            Flatten(path: "product|[Product]") {
              Fetch(service: "d") {
                {
                  ... on Product {
                    __typename
                    isExpensive
                    id
                  }
                } =>
                {
                  ... on Product {
                    canAfford2
                  }
                }
              },
            },
          },
        },
      },
    },
    "###);

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

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "b") {
          {
            product {
              __typename
              id
              hasDiscount
            }
          }
        },
        Flatten(path: "product") {
          Fetch(service: "c") {
            {
              ... on Product {
                __typename
                hasDiscount
                id
              }
            } =>
            {
              ... on Product {
                isExpensiveWithDiscount
              }
            }
          },
        },
        Flatten(path: "product") {
          Fetch(service: "d") {
            {
              ... on Product {
                __typename
                isExpensiveWithDiscount
                id
              }
            } =>
            {
              ... on Product {
                canAffordWithDiscount
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

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "b") {
          {
            product {
              __typename
              id
              hasDiscount
            }
          }
        },
        Flatten(path: "product") {
          Fetch(service: "c") {
            {
              ... on Product {
                __typename
                hasDiscount
                id
              }
            } =>
            {
              ... on Product {
                isExpensiveWithDiscount
              }
            }
          },
        },
        Flatten(path: "product") {
          Fetch(service: "d") {
            {
              ... on Product {
                __typename
                id
                isExpensiveWithDiscount
              }
            } =>
            {
              ... on Product {
                fieldInD
                canAffordWithDiscount
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

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "b") {
          {
            product {
              __typename
              id
              hasDiscount
            }
          }
        },
        Flatten(path: "product") {
          Fetch(service: "c") {
            {
              ... on Product {
                __typename
                hasDiscount
                id
              }
            } =>
            {
              ... on Product {
                isExpensiveWithDiscount
              }
            }
          },
        },
        Flatten(path: "product") {
          Fetch(service: "d") {
            {
              ... on Product {
                __typename
                isExpensiveWithDiscount
                id
              }
            } =>
            {
              ... on Product {
                canAffordWithDiscount2
                canAffordWithDiscount
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

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "b") {
          {
            product {
              __typename
              id
            }
          }
        },
        Flatten(path: "product") {
          Fetch(service: "a") {
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
          Fetch(service: "c") {
            {
              ... on Product {
                __typename
                price
                id
              }
            } =>
            {
              ... on Product {
                isExpensive
              }
            }
          },
        },
        Flatten(path: "product") {
          Fetch(service: "d") {
            {
              ... on Product {
                __typename
                isExpensive
                id
              }
            } =>
            {
              ... on Product {
                canAfford
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

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "b") {
          {
            product {
              __typename
              id
            }
          }
        },
        Flatten(path: "product") {
          Fetch(service: "a") {
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
          Fetch(service: "c") {
            {
              ... on Product {
                __typename
                price
                id
              }
            } =>
            {
              ... on Product {
                isExpensive
              }
            }
          },
        },
        Flatten(path: "product") {
          Fetch(service: "d") {
            {
              ... on Product {
                __typename
                isExpensive
                id
              }
            } =>
            {
              ... on Product {
                canAfford2
                canAfford
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

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "b") {
          {
            product {
              __typename
              id
              hasDiscount
            }
          }
        },
        Parallel {
          Flatten(path: "product") {
            Fetch(service: "c") {
              {
                ... on Product {
                  __typename
                  hasDiscount
                  id
                }
              } =>
              {
                ... on Product {
                  isExpensiveWithDiscount
                }
              }
            },
          },
          Flatten(path: "product") {
            Fetch(service: "a") {
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
        },
        Flatten(path: "product") {
          Fetch(service: "c") {
            {
              ... on Product {
                __typename
                price
                id
              }
            } =>
            {
              ... on Product {
                isExpensive
              }
            }
          },
        },
        Flatten(path: "product") {
          Fetch(service: "d") {
            {
              ... on Product {
                __typename
                isExpensiveWithDiscount
                id
                isExpensive
              }
            } =>
            {
              ... on Product {
                canAffordWithDiscount
                canAfford
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

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "b") {
          {
            product {
              __typename
              id
              hasDiscount
            }
          }
        },
        Parallel {
          Flatten(path: "product") {
            Fetch(service: "c") {
              {
                ... on Product {
                  __typename
                  hasDiscount
                  id
                }
              } =>
              {
                ... on Product {
                  isExpensiveWithDiscount
                }
              }
            },
          },
          Flatten(path: "product") {
            Fetch(service: "a") {
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
        },
        Flatten(path: "product") {
          Fetch(service: "c") {
            {
              ... on Product {
                __typename
                price
                id
              }
            } =>
            {
              ... on Product {
                isExpensive
              }
            }
          },
        },
        Flatten(path: "product") {
          Fetch(service: "d") {
            {
              ... on Product {
                __typename
                isExpensive
                isExpensiveWithDiscount
                id
              }
            } =>
            {
              ... on Product {
                canAffordWithAndWithoutDiscount2
                canAffordWithAndWithoutDiscount
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

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "b") {
          {
            product {
              __typename
              id
              hasDiscount
            }
          }
        },
        Parallel {
          Flatten(path: "product") {
            Fetch(service: "c") {
              {
                ... on Product {
                  __typename
                  hasDiscount
                  id
                }
              } =>
              {
                ... on Product {
                  isExpensiveWithDiscount
                }
              }
            },
          },
          Flatten(path: "product") {
            Fetch(service: "a") {
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
        },
        Flatten(path: "product") {
          Fetch(service: "c") {
            {
              ... on Product {
                __typename
                price
                id
              }
            } =>
            {
              ... on Product {
                isExpensive
              }
            }
          },
        },
        Flatten(path: "product") {
          Fetch(service: "d") {
            {
              ... on Product {
                __typename
                isExpensive
                id
                isExpensiveWithDiscount
              }
            } =>
            {
              ... on Product {
                canAfford2
                canAfford
                canAffordWithDiscount2
                canAffordWithDiscount
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
