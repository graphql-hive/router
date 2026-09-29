use crate::query_planner::{
    tests::testkit::{build_query_plan_with_defaults, init_logger},
    utils::parsing::parse_operation,
};
use std::error::Error;

#[test]
fn requires_with_fragments_on_interfaces() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          userFromA {
            permissions
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/requires-with-fragments.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          {
            userFromA {
              __typename
              id
              profile {
                displayName
                __typename
                ... on GuestAccount {
                  guestToken
                  accountType
                }
                ... on AdminAccount {
                  adminLevel
                  accountType
                }
              }
            }
          }
        },
        Flatten(path: "userFromA") {
          Fetch(service: "b") {
            {
              ... on User {
                __typename
                profile {
                  displayName
                  ... on AdminAccount {
                    accountType
                    adminLevel
                  }
                  ... on GuestAccount {
                    accountType
                    guestToken
                  }
                }
                id
              }
            } =>
            {
              ... on User {
                permissions
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
fn requires_with_union_type_condition() -> Result<(), Box<dyn Error>> {
    init_logger();
    // Media only lives in pricing, so `... on Media` has to keep Book there,
    // otherwise the whole requirement goes away and pricing never gets the title
    let document = parse_operation(
        r#"
        query {
          products {
            price
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/requires-union-type-condition.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "catalog") {
          {
            products {
              __typename
              id
              book {
                title
              }
            }
          }
        },
        Flatten(path: "products.@") {
          Fetch(service: "pricing") {
            {
              ... on Product {
                __typename
                book {
                  title
                }
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
    "#);

    Ok(())
}
