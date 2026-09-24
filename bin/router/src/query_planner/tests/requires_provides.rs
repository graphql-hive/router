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

/// `User.orders @provides(fields: "item { ... on Book { isbn } }")`, where `item` is a
/// `Book | Movie` union. The copy's `item` only leads to the provided `Book`, while the plain
/// `item` also leads to `Movie`. So the plain route has to stay, or `... on Movie { title }` is
/// dropped from the plan.
#[test]
fn provided_union_keeps_members_that_are_not_provided() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        {
          user {
            orders {
              item {
                ... on Book {
                  isbn
                  stock
                }
                ... on Movie {
                  title
                }
              }
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/provides-union-member.supergraph.graphql",
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
                  item {
                    __typename
                    ... on Book {
                      isbn
                      stock
                    }
                    ... on Movie {
                      __typename
                      id
                    }
                  }
                }
              }
            }
          },
        },
        Flatten(path: "user.orders.@.item|[Movie]") {
          Fetch(service: "catalog") {
            {
              ... on Movie {
                __typename
                id
              }
            } =>
            {
              ... on Movie {
                title
              }
            }
          },
        },
      },
    },
    "#);

    Ok(())
}

/// `User.orders @provides(fields: "sku ...")` and `Store.orders @provides(fields: "weight")` both
/// lead to `Order` in `orders`. Each one only provides its own fields, so under `Store.orders`
/// `sku` has to come from `catalog`.
#[test]
fn provides_does_not_leak_to_another_parent() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        {
          store {
            orders {
              sku
              weight
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/provides-scope.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "catalog") {
          {
            store {
              __typename
              id
            }
          }
        },
        Flatten(path: "store") {
          Fetch(service: "orders") {
            {
              ... on Store {
                __typename
                id
              }
            } =>
            {
              ... on Store {
                orders {
                  __typename
                  id
                  weight
                }
              }
            }
          },
        },
        Flatten(path: "store.orders.@") {
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
              }
            }
          },
        },
      },
    },
    "#);

    Ok(())
}

/// `User.pastOrders` returns `Order` like `User.orders`, but has no `@provides`.
#[test]
fn provides_does_not_apply_to_another_field() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        {
          user {
            pastOrders {
              sku
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/provides-scope.supergraph.graphql",
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
                pastOrders {
                  __typename
                  id
                }
              }
            }
          },
        },
        Flatten(path: "user.pastOrders.@") {
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
              }
            }
          },
        },
      },
    },
    "#);

    Ok(())
}

/// `User.orders` copies `ZOrder` before `ZOrder.product @provides(fields: "name")` is handled,
/// because `User` sorts first. The copy still has to get `name` under `product`, `Product` has
/// no key to fetch it from `catalog`.
#[test]
fn provides_reaches_copies_made_earlier() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        {
          user {
            orders {
              sku
              product {
                name
              }
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/provides-copies.supergraph.graphql",
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
                  product {
                    name
                  }
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
