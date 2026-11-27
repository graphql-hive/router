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
                __typename
                displayName
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

/// `permissions @skip(if: $skip)` needs more of `profile` than `basicPermissions` does, and the
/// extra part is only fetched when `$skip` is false. In one shared call, `$skip: true` would send
/// a `profile` with fields missing, it'd become `null` and break `basicPermissions`. So they get
/// separate calls.
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
              ... on User @skip(if: $skip) {
                __typename
                profile {
                  __typename
                  displayName
                  ... on GuestAccount {
                    guestToken
                    accountType
                  }
                  ... on AdminAccount {
                    adminLevel
                    accountType
                  }
                }
                id
              }
              profile {
                __typename
                displayName
              }
            }
          }
        },
        BatchFetch(service: "b") {
          {
            _e0 {
              paths: [
                "userFromA"
              ]
              {
                ... on User {
                  __typename
                  profile {
                    displayName
                  }
                  id
                }
              }
            }
            _e1(if: !$skip) {
              paths: [
                "userFromA"
              ]
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
              }
            }
          }
          {
            _e0: _entities(representations: $__batch_reps_0) {
              ... on User {
                basicPermissions
              }
            }
            _e1: _entities(representations: $__batch_reps_1) {
              ... on User {
                permissions
              }
            }
          }
        },
      },
    },
    "#);

    Ok(())
}

/// Same as above, plus `permissions` on another `userFromA` under `@include`. Neither
/// conditional call shares with `basicPermissions`, and the two conditional ones stay separate
/// too.
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
              ... on User @skip(if: $skip) {
                ...a
              }
              profile {
                __typename
                displayName
              }
            }
            userFromA @include(if: $include) {
              ...a
            }
          }
          fragment a on User {
            __typename
            profile {
              __typename
              displayName
              ... on GuestAccount {
                guestToken
                accountType
              }
              ... on AdminAccount {
                adminLevel
                accountType
              }
            }
            id
          }
        },
        BatchFetch(service: "b") {
          {
            _e0 {
              paths: [
                "userFromA"
              ]
              {
                ... on User {
                  __typename
                  profile {
                    displayName
                  }
                  id
                }
              }
            }
            _e1(if: !$skip) {
              paths: [
                "userFromA"
              ]
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
              }
            }
            _e2(if: $include) {
              paths: [
                "userFromA"
              ]
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
              }
            }
          }
          {
            _e0: _entities(representations: $__batch_reps_0) {
              ... on User {
                basicPermissions
              }
            }
            _e1: _entities(representations: $__batch_reps_1) {
              ... on User {
                ...a
              }
            }
            _e2: _entities(representations: $__batch_reps_2) {
              ... on User {
                ...a
              }
            }
          }
          fragment a on User {
            permissions
          }
        },
      },
    },
    "#);

    Ok(())
}

#[test]
fn merged_conditional_inputs_stay_out_of_an_unconditional_call() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query ($skip: Boolean!, $include: Boolean!) {
          userFromA {
            basicPermissions
            permissions @skip(if: $skip)
            other: permissions @include(if: $include)
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
              ... on User @include(if: $include) {
                ...a
              }
              ... on User @skip(if: $skip) {
                ...a
              }
              profile {
                __typename
                displayName
              }
            }
          }
          fragment a on User {
            __typename
            profile {
              __typename
              displayName
              ... on GuestAccount {
                guestToken
                accountType
              }
              ... on AdminAccount {
                adminLevel
                accountType
              }
            }
            id
          }
        },
        BatchFetch(service: "b") {
          {
            _e0 {
              paths: [
                "userFromA"
              ]
              {
                ... on User {
                  __typename
                  profile {
                    displayName
                  }
                  id
                }
              }
            }
            _e1(if: !$skip || $include) {
              paths: [
                "userFromA"
              ]
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
              }
            }
          }
          ($include:Boolean!,$skip:Boolean!) {
            _e0: _entities(representations: $__batch_reps_0) {
              ... on User {
                basicPermissions
              }
            }
            _e1: _entities(representations: $__batch_reps_1) {
              ... on User {
                ... on User @skip(if: $skip) {
                  permissions
                }
                ... on User @include(if: $include) {
                  other: permissions
                }
              }
            }
          }
        },
      },
    },
    "#);

    Ok(())
}

/// `computed` requires fields of both union members. An `A` gets its key for `x` from `y`, a
/// `B` gets its key for `y` from `x`. One call per subgraph for both types would wait on itself.
#[test]
fn requires_over_union_members_with_crossed_keys() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          holder {
            computed
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/requires-union-crossed-keys.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "r") {
          {
            holder {
              __typename
              id
              u {
                __typename
                ... on B {
                  __typename
                  id
                }
                ... on A {
                  __typename
                  id
                }
              }
            }
          }
        },
        Parallel {
          Flatten(path: "holder.u.@|[B]") {
            Fetch(service: "x") {
              {
                ... on B {
                  __typename
                  id
                }
              } =>
              {
                ... on B {
                  yk
                }
              }
            },
          },
          Flatten(path: "holder.u.@|[A]") {
            Fetch(service: "y") {
              {
                ... on A {
                  __typename
                  id
                }
              } =>
              {
                ... on A {
                  xk
                }
              }
            },
          },
        },
        Parallel {
          Flatten(path: "holder.u.@|[B]") {
            Fetch(service: "y") {
              {
                ... on B {
                  __typename
                  yk
                }
              } =>
              {
                ... on B {
                  bField
                }
              }
            },
          },
          Flatten(path: "holder.u.@|[A]") {
            Fetch(service: "x") {
              {
                ... on A {
                  __typename
                  xk
                }
              } =>
              {
                ... on A {
                  aField
                }
              }
            },
          },
        },
        Flatten(path: "holder") {
          Fetch(service: "z") {
            {
              ... on Holder {
                __typename
                u {
                  ... on A {
                    aField
                  }
                  ... on B {
                    bField
                  }
                }
                id
              }
            } =>
            {
              ... on Holder {
                computed
              }
            }
          },
        },
      },
    },
    "#);

    Ok(())
}
