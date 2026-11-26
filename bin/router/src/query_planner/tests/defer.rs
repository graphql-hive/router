//! `@defer` plans, see `ast::defer`. A fetch with `defer: N` is for the deferred part `N`, it
//! runs once the fetches it reads from are done. The rest are for the primary part.

use std::error::Error;

use crate::query_planner::tests::testkit::{
    build_defer_display_plan, build_query_plan_with_defaults, init_logger,
};
use crate::query_planner::utils::parsing::parse_operation;

const FIXTURE: &str = "fixture/products-example.supergraph.graphql";

#[test]
fn deferred_fields_from_another_subgraph_get_their_own_fetch() -> Result<(), Box<dyn Error>> {
    init_logger();
    let plan = build_query_plan_with_defaults(
        FIXTURE,
        parse_operation(
            r#"
            {
              topProducts {
                upc
                name
                ... @defer {
                  reviews { body }
                }
              }
            }
            "#,
        ),
    )?;
    insta::assert_snapshot!(format!("{}", plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "products") {
          {
            topProducts {
              __typename
              upc
              name
            }
          }
        },
        Flatten(path: "topProducts.@") {
          Fetch(service: "reviews", defer: 0) {
            {
              ... on Product {
                __typename
                upc
              }
            } =>
            {
              ... on Product {
                reviews {
                  body
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
fn deferred_fields_from_the_same_subgraph_get_their_own_entity_call() -> Result<(), Box<dyn Error>>
{
    init_logger();
    let plan = build_query_plan_with_defaults(
        FIXTURE,
        parse_operation(
            r#"
            {
              topProducts {
                upc
                ... @defer { name }
              }
            }
            "#,
        ),
    )?;
    insta::assert_snapshot!(format!("{}", plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "products") {
          {
            topProducts {
              __typename
              upc
            }
          }
        },
        Flatten(path: "topProducts.@") {
          Fetch(service: "products", defer: 0) {
            {
              ... on Product {
                __typename
                upc
              }
            } =>
            {
              ... on Product {
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

#[test]
fn nested_defers_get_their_own_calls() -> Result<(), Box<dyn Error>> {
    init_logger();
    let plan = build_query_plan_with_defaults(
        FIXTURE,
        parse_operation(
            r#"
            {
              topProducts {
                upc
                ... @defer(label: "reviews") {
                  reviews {
                    body
                    author {
                      id
                      ... @defer(label: "author") { name }
                    }
                  }
                }
              }
            }
            "#,
        ),
    )?;
    insta::assert_snapshot!(format!("{}", plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "products") {
          {
            topProducts {
              __typename
              upc
            }
          }
        },
        Flatten(path: "topProducts.@") {
          Fetch(service: "reviews", defer: 0) {
            {
              ... on Product {
                __typename
                upc
              }
            } =>
            {
              ... on Product {
                reviews {
                  body
                  author {
                    __typename
                    id
                  }
                }
              }
            }
          },
        },
        Flatten(path: "topProducts.@.reviews.@.author") {
          Fetch(service: "accounts", defer: 1) {
            {
              ... on User {
                __typename
                id
              }
            } =>
            {
              ... on User {
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

#[test]
fn deferred_root_field() -> Result<(), Box<dyn Error>> {
    init_logger();
    let plan = build_query_plan_with_defaults(
        FIXTURE,
        parse_operation(
            r#"
            {
              topProducts { upc }
              ... @defer { me { id name } }
            }
            "#,
        ),
    )?;
    insta::assert_snapshot!(format!("{}", plan), @r#"
    QueryPlan {
      Parallel {
        Fetch(service: "accounts", defer: 0) {
          {
            me {
              id
              name
            }
          }
        },
        Fetch(service: "products") {
          {
            topProducts {
              upc
            }
          }
        },
      },
    },
    "#);
    Ok(())
}

#[test]
fn conditional_fragment_at_the_root_has_no_type_condition() -> Result<(), Box<dyn Error>> {
    init_logger();
    let plan = build_query_plan_with_defaults(
        FIXTURE,
        parse_operation(
            r#"
            query ($x: Boolean!) {
              me { id }
              ... @include(if: $x) { me { name } }
            }
            "#,
        ),
    )?;
    // a subgraph can name its root type something else, so the fragment has no `on Query`
    let json = serde_json::to_string(&plan)?;
    assert!(json.contains(r#"... @include(if: $x){me{name}}"#), "{json}");
    Ok(())
}

#[test]
fn defer_with_include_keeps_the_include() -> Result<(), Box<dyn Error>> {
    init_logger();
    let plan = build_query_plan_with_defaults(
        FIXTURE,
        parse_operation(
            r#"
            query ($i: Boolean!) {
              topProducts {
                upc
                ... @include(if: $i) @defer {
                  reviews { body }
                }
              }
            }
            "#,
        ),
    )?;
    insta::assert_snapshot!(format!("{}", plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "products") {
          query ($i:Boolean!) {
            topProducts {
              upc
              ... on Product @include(if: $i) {
                __typename
                upc
              }
            }
          }
        },
        Flatten(path: "topProducts.@|[Product]") {
          Fetch(service: "reviews", defer: 0, if: $i) {
            {
              ... on Product {
                __typename
                upc
              }
            } =>
            {
              ... on Product {
                reviews {
                  body
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

/// How plans with `@defer` look when exposed: `Defer`, `Primary` and `Deferred`.
#[test]
fn display_plans() -> Result<(), Box<dyn Error>> {
    init_logger();
    let show = |query: &str| -> Result<String, Box<dyn Error>> {
        let plan = build_defer_display_plan(FIXTURE, parse_operation(query))?;
        Ok(format!("{plan}\n{}", sonic_rs::to_string_pretty(&plan)?))
    };

    insta::assert_snapshot!(show(
        r#"{ topProducts { upc ... @defer(label: "reviews") { reviews { body } } } }"#
    )?, @r#"
    QueryPlan {
      Defer {
        Primary {
          { topProducts { upc } }:
          Fetch(service: "products", id: 2) {
            {topProducts{__typename upc}}
          },
        }, [
          Deferred(depends: [2], path: "topProducts.@", label: "reviews") {
            { reviews { body } }:
            Flatten(path: "topProducts.@") {
              Fetch(service: "reviews", id: 3) {
                {
                  ... on Product {
                    __typename
                    upc
                  }
                } =>
                query($representations:[_Any!]!){_entities(representations: $representations){...on Product{reviews{body}}}}
              },
            },
          },
        ]
      },
    },

    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Defer",
        "primary": {
          "subselection": "{ topProducts { upc } }",
          "node": {
            "kind": "Fetch",
            "id": 2,
            "serviceName": "products",
            "operationKind": "query",
            "operation": "{topProducts{__typename upc}}"
          }
        },
        "deferred": [
          {
            "depends": [
              {
                "id": "2"
              }
            ],
            "label": "reviews",
            "queryPath": [
              "topProducts",
              "@"
            ],
            "subselection": "{ reviews { body } }",
            "node": {
              "kind": "Flatten",
              "path": [
                {
                  "Field": "topProducts"
                },
                "@"
              ],
              "node": {
                "kind": "Fetch",
                "id": 3,
                "serviceName": "reviews",
                "operationKind": "query",
                "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Product{reviews{body}}}}",
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
            }
          }
        ]
      }
    }
    "#);
    insta::assert_snapshot!(show(
        r#"{ topProducts { upc ... @defer(label: "reviews") { reviews { body author { id ... @defer(label: "author") { name } } } } } }"#
    )?, @r#"
    QueryPlan {
      Defer {
        Primary {
          { topProducts { upc } }:
          Fetch(service: "products", id: 2) {
            {topProducts{__typename upc}}
          },
        }, [
          Deferred(depends: [2], path: "topProducts.@", label: "reviews") {
            Defer {
              Primary {
                { reviews { body author { id } } }:
                Flatten(path: "topProducts.@") {
                  Fetch(service: "reviews", id: 3) {
                    {
                      ... on Product {
                        __typename
                        upc
                      }
                    } =>
                    query($representations:[_Any!]!){_entities(representations: $representations){...on Product{reviews{body author{__typename id}}}}}
                  },
                },
              }, [
                Deferred(depends: [3], path: "topProducts.@.reviews.@.author", label: "author") {
                  { name }:
                  Flatten(path: "topProducts.@.reviews.@.author") {
                    Fetch(service: "accounts", id: 4) {
                      {
                        ... on User {
                          __typename
                          id
                        }
                      } =>
                      query($representations:[_Any!]!){_entities(representations: $representations){...on User{name}}}
                    },
                  },
                },
              ]
            },
          },
        ]
      },
    },

    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Defer",
        "primary": {
          "subselection": "{ topProducts { upc } }",
          "node": {
            "kind": "Fetch",
            "id": 2,
            "serviceName": "products",
            "operationKind": "query",
            "operation": "{topProducts{__typename upc}}"
          }
        },
        "deferred": [
          {
            "depends": [
              {
                "id": "2"
              }
            ],
            "label": "reviews",
            "queryPath": [
              "topProducts",
              "@"
            ],
            "node": {
              "kind": "Defer",
              "primary": {
                "subselection": "{ reviews { body author { id } } }",
                "node": {
                  "kind": "Flatten",
                  "path": [
                    {
                      "Field": "topProducts"
                    },
                    "@"
                  ],
                  "node": {
                    "kind": "Fetch",
                    "id": 3,
                    "serviceName": "reviews",
                    "operationKind": "query",
                    "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Product{reviews{body author{__typename id}}}}}",
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
                }
              },
              "deferred": [
                {
                  "depends": [
                    {
                      "id": "3",
                      "deferLabel": "reviews"
                    }
                  ],
                  "label": "author",
                  "queryPath": [
                    "topProducts",
                    "@",
                    "reviews",
                    "@",
                    "author"
                  ],
                  "subselection": "{ name }",
                  "node": {
                    "kind": "Flatten",
                    "path": [
                      {
                        "Field": "topProducts"
                      },
                      "@",
                      {
                        "Field": "reviews"
                      },
                      "@",
                      {
                        "Field": "author"
                      }
                    ],
                    "node": {
                      "kind": "Fetch",
                      "id": 4,
                      "serviceName": "accounts",
                      "operationKind": "query",
                      "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on User{name}}}",
                      "requires": [
                        {
                          "kind": "InlineFragment",
                          "typeCondition": "User",
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
fn deferred_fields_under_include_get_their_own_entity_call() -> Result<(), Box<dyn Error>> {
    init_logger();
    let plan = build_query_plan_with_defaults(
        FIXTURE,
        parse_operation(
            r#"
            query ($i: Boolean!) {
              topProducts {
                upc
                ... @include(if: $i) @defer {
                  name
                }
              }
            }
            "#,
        ),
    )?;
    insta::assert_snapshot!(format!("{}", plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "products") {
          query ($i:Boolean!) {
            topProducts {
              upc
              ... on Product @include(if: $i) {
                __typename
                upc
              }
            }
          }
        },
        Flatten(path: "topProducts.@|[Product]") {
          Fetch(service: "products", defer: 0, if: $i) {
            {
              ... on Product {
                __typename
                upc
              }
            } =>
            {
              ... on Product {
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

/// An entity call under a deferred one, with an `@include` of its own, still waits for the
/// `@defer`: it reads what the deferred call fetched.
#[test]
fn nested_condition_keeps_the_defer_of_the_call_it_reads_from() -> Result<(), Box<dyn Error>> {
    init_logger();
    let plan = |query: &str| -> Result<String, Box<dyn Error>> {
        Ok(build_query_plan_with_defaults(FIXTURE, parse_operation(query))?.to_string())
    };
    insta::assert_snapshot!(plan(
        r#"query ($v: Boolean!) { me { id } ... @defer { me { reviews @include(if: $v) { body } } } }"#
    )?, @r#"
    QueryPlan {
      Sequence {
        Parallel {
          Fetch(service: "accounts", defer: 0) {
            {
              me {
                __typename
                id
              }
            }
          },
          Fetch(service: "accounts") {
            {
              me {
                id
              }
            }
          },
        },
        Flatten(path: "me") {
          Fetch(service: "reviews", defer: 0, if: $v) {
            {
              ... on User {
                __typename
                id
              }
            } =>
            {
              ... on User {
                reviews {
                  body
                }
              }
            }
          },
        },
      },
    },
    "#);
    insta::assert_snapshot!(plan(
        r#"query ($v: Boolean!) { topProducts { upc ... @defer { reviews { author @include(if: $v) { name } } } } }"#
    )?, @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "products") {
          {
            topProducts {
              __typename
              upc
            }
          }
        },
        Flatten(path: "topProducts.@") {
          Fetch(service: "reviews", defer: 0) {
            {
              ... on Product {
                __typename
                upc
              }
            } =>
            ($v:Boolean!) {
              ... on Product {
                reviews {
                  author @include(if: $v) {
                    __typename
                    id
                  }
                }
              }
            }
          },
        },
        Flatten(path: "topProducts.@.reviews.@.author") {
          Fetch(service: "accounts", defer: 0, if: $v) {
            {
              ... on User {
                __typename
                id
              }
            } =>
            {
              ... on User {
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

/// The same field twice, once under an `@include`, with a `@defer` only in the conditional one:
/// the deferred call keeps both conditions.
#[test]
fn deferred_call_under_a_conditional_copy_of_a_field_keeps_its_conditions(
) -> Result<(), Box<dyn Error>> {
    init_logger();
    let plan = build_query_plan_with_defaults(
        FIXTURE,
        parse_operation(
            r#"query ($v: Boolean!) {
              me {
                reviews { id author { birthday } }
                reviews @include(if: $v) { ... @defer { author { username } } }
              }
            }"#,
        ),
    )?;
    insta::assert_snapshot!(format!("{}", plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "accounts") {
          {
            me {
              __typename
              id
            }
          }
        },
        Flatten(path: "me") {
          Fetch(service: "reviews") {
            {
              ... on User {
                __typename
                id
              }
            } =>
            ($v:Boolean!) {
              ... on User {
                ... on User @include(if: $v) {
                  reviews {
                    __typename
                    id
                  }
                }
                reviews {
                  id
                  author {
                    __typename
                    id
                  }
                }
              }
            }
          },
        },
        Parallel {
          Flatten(path: "me.reviews.@.author") {
            Fetch(service: "accounts") {
              {
                ... on User {
                  __typename
                  id
                }
              } =>
              {
                ... on User {
                  birthday
                }
              }
            },
          },
          Flatten(path: "me.reviews.@") {
            Fetch(service: "reviews", defer: 0, if: $v) {
              {
                ... on Review {
                  __typename
                  id
                }
              } =>
              {
                ... on Review {
                  author {
                    username
                  }
                }
              }
            },
          },
        },
      },
    },
    "#);
    Ok(())
}

#[test]
fn deferred_calls_under_a_skipped_copy_and_a_plain_copy_of_a_field() -> Result<(), Box<dyn Error>> {
    init_logger();
    let plan = build_query_plan_with_defaults(
        FIXTURE,
        parse_operation(
            r#"query ($v: Boolean!) {
              user(id: "2") {
                reviews @skip(if: $v) { ... @defer { author { birthday } } }
                reviews { author { id } ... @defer { body } }
              }
            }"#,
        ),
    )?;
    insta::assert_snapshot!(format!("{}", plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "accounts") {
          {
            user(id: "2") {
              __typename
              id
            }
          }
        },
        Flatten(path: "user") {
          Fetch(service: "reviews") {
            {
              ... on User {
                __typename
                id
              }
            } =>
            ($v:Boolean!) {
              ... on User {
                reviews {
                  __typename
                  author {
                    id
                  }
                  id
                }
                ... on User @skip(if: $v) {
                  reviews {
                    __typename
                    id
                  }
                }
              }
            }
          },
        },
        Parallel {
          Flatten(path: "user.reviews.@") {
            Fetch(service: "reviews", defer: 0, if: !$v) {
              {
                ... on Review {
                  __typename
                  id
                }
              } =>
              {
                ... on Review {
                  author {
                    __typename
                    id
                  }
                }
              }
            },
          },
          Flatten(path: "user.reviews.@") {
            Fetch(service: "reviews", defer: 1) {
              {
                ... on Review {
                  __typename
                  id
                }
              } =>
              {
                ... on Review {
                  body
                }
              }
            },
          },
        },
        Flatten(path: "user.reviews.@.author") {
          Fetch(service: "accounts", defer: 0, if: !$v) {
            {
              ... on User {
                __typename
                id
              }
            } =>
            {
              ... on User {
                birthday
              }
            }
          },
        },
      },
    },
    "#);
    Ok(())
}

/// Entity calls to one subgraph that run at the same time are batched into one request, but
/// only within a part. Here both `reviews` calls are in the same deferred part, so they're one
/// batch for that part.
#[test]
fn entity_calls_of_one_deferred_part_are_batched() -> Result<(), Box<dyn Error>> {
    init_logger();
    let plan = build_query_plan_with_defaults(
        FIXTURE,
        parse_operation(
            r#"
            {
              topProducts { upc }
              me { id }
              ... @defer {
                topProducts { reviews { body } }
                me { reviews { body } }
              }
            }
            "#,
        ),
    )?;
    insta::assert_snapshot!(format!("{}", plan), @r#"
    QueryPlan {
      Sequence {
        Parallel {
          Fetch(service: "accounts", defer: 0) {
            {
              me {
                __typename
                id
              }
            }
          },
          Fetch(service: "accounts") {
            {
              me {
                id
              }
            }
          },
          Fetch(service: "products", defer: 0) {
            {
              topProducts {
                __typename
                upc
              }
            }
          },
          Fetch(service: "products") {
            {
              topProducts {
                upc
              }
            }
          },
        },
        BatchFetch(service: "reviews", defer: 0) {
          {
            _e0 {
              paths: [
                "me"
              ]
              {
                ... on User {
                  __typename
                  id
                }
              }
            }
            _e1 {
              paths: [
                "topProducts.@"
              ]
              {
                ... on Product {
                  __typename
                  upc
                }
              }
            }
          }
          {
            _e0: _entities(representations: $__batch_reps_0) {
              ... on User {
                reviews {
                  ...a
                }
              }
            }
            _e1: _entities(representations: $__batch_reps_1) {
              ... on Product {
                reviews {
                  ...a
                }
              }
            }
          }
          fragment a on Review {
            body
          }
        },
      },
    },
    "#);
    Ok(())
}

/// The same two `reviews` calls, each in its own deferred part. A batch can't span parts, a
/// part has to be able to go out without waiting for the other, so they stay two calls.
#[test]
fn entity_calls_of_different_deferred_parts_are_not_batched() -> Result<(), Box<dyn Error>> {
    init_logger();
    let plan = build_query_plan_with_defaults(
        FIXTURE,
        parse_operation(
            r#"
            {
              topProducts { upc ... @defer { reviews { body } } }
              me { id ... @defer { reviews { body } } }
            }
            "#,
        ),
    )?;
    insta::assert_snapshot!(format!("{}", plan), @r#"
    QueryPlan {
      Sequence {
        Parallel {
          Fetch(service: "accounts") {
            {
              me {
                __typename
                id
              }
            }
          },
          Fetch(service: "products") {
            {
              topProducts {
                __typename
                upc
              }
            }
          },
        },
        Parallel {
          Flatten(path: "me") {
            Fetch(service: "reviews", defer: 1) {
              {
                ... on User {
                  __typename
                  id
                }
              } =>
              {
                ... on User {
                  reviews {
                    body
                  }
                }
              }
            },
          },
          Flatten(path: "topProducts.@") {
            Fetch(service: "reviews", defer: 0) {
              {
                ... on Product {
                  __typename
                  upc
                }
              } =>
              {
                ... on Product {
                  reviews {
                    body
                  }
                }
              }
            },
          },
        },
      },
    },
    "#);
    Ok(())
}
