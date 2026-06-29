use crate::{
    tests::testkit::{build_query_plan_with_defaults, init_logger},
    utils::parsing::parse_operation,
};
use std::error::Error;

#[test]
pub fn mutation_referencing_query() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        mutation {
          doSomething {
            ok
            query {
              healthCheck
              user(id: 1) {
                name
              }
            }
          }
        }
        "#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/tests/root-reentry.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "two") {
          mutation {
            doSomething {
              ok
              query {
                healthCheck
                __typename
              }
            }
          }
        },
        Flatten(path: "doSomething.query") {
          Fetch(service: "one") {
            {
              user(id: 1) {
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
pub fn mutation_referencing_query_with_alias() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        mutation {
          doSomething {
            ok
            info: query {
              healthCheck
              user(id: 1) {
                name
              }
            }
          }
        }
        "#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/tests/root-reentry.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "two") {
          mutation {
            doSomething {
              ok
              info: query {
                healthCheck
                __typename
              }
            }
          }
        },
        Flatten(path: "doSomething.info") {
          Fetch(service: "one") {
            {
              user(id: 1) {
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
pub fn mutation_referencing_query_with_condition() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        mutation($withQuery: Boolean!) {
          doSomething {
            ok
            query @include(if: $withQuery) {
              healthCheck
              user(id: 1) {
                name
              }
            }
          }
        }
        "#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/tests/root-reentry.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "two") {
          mutation ($withQuery:Boolean!) {
            doSomething {
              ok
              query @include(if: $withQuery) {
                healthCheck
                __typename
              }
            }
          }
        },
        Include(if: $withQuery) {
          Flatten(path: "doSomething.query") {
            Fetch(service: "one") {
              {
                user(id: 1) {
                  name
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
pub fn query_referencing_query() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          healthCheck
          user(id: 1) {
            name
          }
          system {
            healthCheck
          }
        }
        "#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/tests/root-reentry.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Parallel {
        Fetch(service: "one") {
          {
            user(id: 1) {
              name
            }
          }
        },
        Fetch(service: "two") {
          {
            healthCheck
            system {
              healthCheck
            }
          }
        },
      },
    },
    "#);

    Ok(())
}

#[test]
pub fn query_referencing_query_cross_subgraphs() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          healthCheck
          user(id: 1) {
            name
          }
          system {
            healthCheck
            user(id: 1) {
              name
            }
            system {
              healthCheck
              user(id: 1) {
                name
              }
              system {
                healthCheck
              }
            }
          }
        }
        "#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/tests/root-reentry.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Parallel {
        Fetch(service: "one") {
          {
            user(id: 1) {
              ...a
            }
            system {
              user(id: 1) {
                ...a
              }
              system {
                user(id: 1) {
                  ...a
                }
              }
            }
          }
          fragment a on User {
            name
          }
        },
        Fetch(service: "two") {
          {
            healthCheck
            system {
              healthCheck
              system {
                healthCheck
                system {
                  healthCheck
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

#[test]
pub fn query_referncing_through_inner() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          user(id: 1) {
            id
            name
          }
          inner {
            toQuery {
              user(id: 1) {
                id
                name
              }
            }
            toMutation {
              doSomething {
                ok
                query {
                  healthCheck
                  user(id: 1) {
                    name
                  }
                }
              }
            }
          }
        }
        "#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/tests/root-reentry.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Parallel {
          Fetch(service: "two") {
            {
              inner {
                toMutation {
                  doSomething {
                    ok
                    query {
                      healthCheck
                      __typename
                    }
                  }
                }
              }
            }
          },
          Fetch(service: "one") {
            {
              user(id: 1) {
                ...a
              }
              inner {
                toQuery {
                  user(id: 1) {
                    ...a
                  }
                }
              }
            }
            fragment a on User {
              id
              name
            }
          },
        },
        Flatten(path: "inner.toMutation.doSomething.query") {
          Fetch(service: "one") {
            {
              user(id: 1) {
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

// Start with a mutation to subgraph "two", then all re-entry fields are resolved from "one"
#[test]
pub fn mutation_then_full_reentry_jump() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
      mutation {
        doSomething { # two
          query { # jump to one
            user(id: 1) { # one
              name # one
            }
          }
        }
      }
      "#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/tests/root-reentry.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "two") {
          mutation {
            doSomething {
              query {
                __typename
              }
            }
          }
        },
        Flatten(path: "doSomething.query") {
          Fetch(service: "one") {
            {
              user(id: 1) {
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

// Start with a mutation to subgraph "two", then all mixed fetch from both subgraphs
#[test]
pub fn mutation_then_mixed_query() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
      mutation {
        doSomething { # two
          ok # two
          query { # one+two
            healthCheck # two
            user(id: 1) { # one
              name # one
            }
          }
        }
      }
      "#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/tests/root-reentry.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "two") {
          mutation {
            doSomething {
              ok
              query {
                healthCheck
                __typename
              }
            }
          }
        },
        Flatten(path: "doSomething.query") {
          Fetch(service: "one") {
            {
              user(id: 1) {
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
