use crate::{
    tests::testkit::{build_query_plan, init_logger},
    utils::parsing::parse_operation,
};
use std::error::Error;

#[test]
fn issue_281_test() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        {
          viewer {
            review {
              ... on AnonymousReview {
                __typename
                product {
                  b
                }
              }
              ... on UserReview {
                __typename
                product {
                  c
                  d
                }
              }
            }
          }
        }

        "#,
    );
    let query_plan = build_query_plan("fixture/issues/281.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r###"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          {
            viewer {
              review {
                __typename
                ... on AnonymousReview {
                  __typename
                  product {
    ...a              }
                }
                ... on UserReview {
                  __typename
                  product {
    ...a              }
                }
              }
            }
          }
          fragment a on Product {
            __typename
            id
          }
        },
        Flatten(path: "viewer.review|[UserReview].product") {
          Fetch(service: "b") {
            {
              ... on Product {
                __typename
                id
              }
            } =>
            {
              ... on Product {
                pid
                b
              }
            }
          },
        },
        Flatten(path: "viewer.review|[UserReview].product") {
          Fetch(service: "c") {
            {
              ... on Product {
                __typename
                pid
              }
            } =>
            {
              ... on Product {
                c
                pid
              }
            }
          },
        },
        Flatten(path: "viewer.review|[UserReview].product") {
          Fetch(service: "d") {
            {
              ... on Product {
                __typename
                pid
              }
            } =>
            {
              ... on Product {
                d
              }
            }
          },
        },
      },
    },
    "###);

    Ok(())
}
