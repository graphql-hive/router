use crate::{
    tests::testkit::{build_query_plan, init_logger},
    utils::parsing::parse_operation,
};
use std::error::Error;

#[test]
fn requires_circular_1() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        {
          feed {
            byNovice
          }
        }
        "#,
    );
    let query_plan = build_query_plan(
        "fixture/tests/requires-circular.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          {
            feed {
              __typename
              id
            }
          }
        },
        Flatten(path: "feed.@") {
          Fetch(service: "b") {
            {
              ... on Post {
                __typename
                id
              }
            } =>
            {
              ... on Post {
                author {
                  __typename
                  id
                }
              }
            }
          },
        },
        Flatten(path: "feed.@.author") {
          Fetch(service: "a") {
            {
              ... on Author {
                __typename
                id
              }
            } =>
            {
              ... on Author {
                yearsOfExperience
              }
            }
          },
        },
        Flatten(path: "feed.@") {
          Fetch(service: "b") {
            {
              ... on Post {
                __typename
                author {
                  yearsOfExperience
                }
                id
              }
            } =>
            {
              ... on Post {
                byNovice
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
fn requires_circular_2() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        {
          feed {
            byExpert
          }
        }
        "#,
    );
    let query_plan = build_query_plan(
        "fixture/tests/requires-circular.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          {
            feed {
              __typename
              id
            }
          }
        },
        Flatten(path: "feed.@") {
          Fetch(service: "b") {
            {
              ... on Post {
                __typename
                id
              }
            } =>
            {
              ... on Post {
                author {
                  __typename
                  id
                }
              }
            }
          },
        },
        Flatten(path: "feed.@.author") {
          Fetch(service: "a") {
            {
              ... on Author {
                __typename
                id
              }
            } =>
            {
              ... on Author {
                yearsOfExperience
              }
            }
          },
        },
        Flatten(path: "feed.@") {
          Fetch(service: "b") {
            {
              ... on Post {
                __typename
                author {
                  yearsOfExperience
                }
                id
              }
            } =>
            {
              ... on Post {
                byNovice
              }
            }
          },
        },
        Flatten(path: "feed.@") {
          Fetch(service: "a") {
            {
              ... on Post {
                __typename
                byNovice
                id
              }
            } =>
            {
              ... on Post {
                byExpert
              }
            }
          },
        },
      },
    },
    "#);

    Ok(())
}
