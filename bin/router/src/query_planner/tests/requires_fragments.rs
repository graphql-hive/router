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

#[test]
fn requires_root_fragment_on_inapplicable_type() -> Result<(), Box<dyn Error>> {
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
        "fixture/tests/requires-inapplicable-root-fragment.supergraph.graphql",
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

    Ok(())
}

/// `permissions` under `@skip` needs more of `profile` than `basicPermissions`, and that's only
/// fetched when `$skip` is off. One call for both would send a `profile` missing those fields
/// with `$skip` on, so `null`, and `basicPermissions` would come out wrong. They go out apart.
#[test]
fn conditional_requires_stay_out_of_an_unconditional_call() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query ($skip: Boolean!) {
          userFromA {
            basicPermissions
            permissions @skip(if: $skip)
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
          query ($skip:Boolean!) {
            userFromA {
              __typename
              id
              profile {
                displayName
              }
              ... on User @skip(if: $skip) {
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
                __typename
                id
              }
            }
          }
        },
        Parallel {
          Skip(if: $skip) {
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
          Flatten(path: "userFromA") {
            Fetch(service: "b") {
              {
                ... on User {
                  __typename
                  profile {
                    displayName
                  }
                  id
                }
              } =>
              {
                ... on User {
                  basicPermissions
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

/// Same, with `permissions` also under `@include`. The two conditional calls share their input,
/// merged they're under no condition any more, but their input still is.
#[test]
fn merged_conditional_requires_stay_out_of_an_unconditional_call() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query ($skip: Boolean!, $include: Boolean!) {
          userFromA {
            basicPermissions
            permissions @skip(if: $skip)
          }
          userFromA @include(if: $include) {
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
          query ($include:Boolean!,$skip:Boolean!) {
            userFromA {
              __typename
              id
              profile {
                displayName
              }
              ... on User @skip(if: $skip) {
                ...a
              }
            }
            userFromA @include(if: $include) {
              ...a
            }
          }
          fragment a on User {
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
            __typename
            id
          }
        },
        Parallel {
          Include(if: $include) {
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
          Skip(if: $skip) {
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
          Flatten(path: "userFromA") {
            Fetch(service: "b") {
              {
                ... on User {
                  __typename
                  profile {
                    displayName
                  }
                  id
                }
              } =>
              {
                ... on User {
                  basicPermissions
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
