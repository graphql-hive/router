use crate::{
    tests::testkit::{build_query_plan, init_logger},
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
    let query_plan = build_query_plan(
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
