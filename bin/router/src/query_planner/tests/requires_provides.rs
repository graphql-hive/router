use crate::query_planner::{
    tests::testkit::{build_query_plan_with_defaults, init_logger},
    utils::parsing::parse_operation,
};
use std::error::Error;

#[test]
fn simple_requires_provides() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          me {
            reviews {
              id
              author {
                id
                username
              }
              product {
                inStock
              }
            }
          }
        }"#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/requires-provides.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
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
            {
              ... on User {
                reviews {
                  id
                  author {
                    id
                    username
                  }
                  product {
                    __typename
                    upc
                  }
                }
              }
            }
          },
        },
        Flatten(path: "me.reviews.@.product") {
          Fetch(service: "inventory") {
            {
              ... on Product {
                __typename
                upc
              }
            } =>
            {
              ... on Product {
                inStock
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
            "serviceName": "accounts",
            "operationKind": "query",
            "operation": "{me{__typename id}}"
          },
          {
            "kind": "Flatten",
            "path": [
              {
                "Field": "me"
              }
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "reviews",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on User{reviews{id author{id username} product{__typename upc}}}}}",
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
          },
          {
            "kind": "Flatten",
            "path": [
              {
                "Field": "me"
              },
              {
                "Field": "reviews"
              },
              "@",
              {
                "Field": "product"
              }
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "inventory",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Product{inStock}}}",
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
        ]
      }
    }
    "#);
    Ok(())
}

// https://github.com/graphql-hive/router/issues/1455
#[test]
fn provides_fieldset_with_typename_and_requires() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          review {
            author {
              username
              __typename
            }
          }
        }"#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/provides-requires-typename.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "reviews") {
          {
            review {
              __typename
              id
            }
          }
        },
        Flatten(path: "review") {
          Fetch(service: "secrets") {
            {
              ... on Review {
                __typename
                id
              }
            } =>
            {
              ... on Review {
                secret
              }
            }
          },
        },
        Flatten(path: "review") {
          Fetch(service: "reviews") {
            {
              ... on Review {
                __typename
                secret
                id
              }
            } =>
            {
              ... on Review {
                author {
                  username
                  __typename
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

/// `User.orders @provides(fields: "sku")`, so `orders` can return `sku` for those orders, and
/// `Order.name @requires(fields: "sku")` lives in `orders` too. So both come from the same
/// fetch: `orders { sku name }`.
///
/// We used to ignore the provided `sku`, fetch it from `catalog`, and then go back to `orders`
/// for `name` - 4 fetches instead of 2.
#[test]
fn requires_provided_after_entity_hop() -> Result<(), Box<dyn Error>> {
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
        "fixture/tests/requires-provided-after-hop.supergraph.graphql",
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
                  sku
                  name
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

/// Like `requires_provided_after_entity_hop`, but `name` requires `sku weight` and only `sku` is
/// provided. `weight` needs an entity call to `catalog` anyway, so `name` has to go through the
/// usual `@requires` steps.
///
/// The provided `sku` must not end up in an `_entities` call to `orders`: `sku` is `@external`
/// there, so `orders` can only return it under `User.orders`. The `@include` keeps the
/// requirement fetch from being merged into its parent, which is what used to hide this.
#[test]
fn requires_partly_provided_after_entity_hop() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query ($x: Boolean!) {
          user {
            orders {
              name @include(if: $x)
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/requires-partly-provided-after-hop.supergraph.graphql",
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
                }
              }
            }
          },
        },
        Include(if: $x) {
          Sequence {
            Flatten(path: "user.orders.@") {
              Fetch(service: "catalog") {
                {
                  ... on Order {
                    __typename
                    id
                  }
                } =>
                {
                  ... on Order {
                    sku
                    weight
                  }
                }
              },
            },
            Flatten(path: "user.orders.@") {
              Fetch(service: "orders") {
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
      },
    },
    "#);

    Ok(())
}
