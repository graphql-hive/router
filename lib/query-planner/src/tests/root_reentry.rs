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
