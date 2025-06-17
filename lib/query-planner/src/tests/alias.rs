use crate::{
    tests::testkit::{build_query_plan, init_logger},
    utils::parsing::parse_operation,
};
use std::error::Error;

#[test]
fn aliasing_both_parent_and_leaf() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
            query {
              allProducts: products {
                price {
                  pricing: amount
                  currency
                }
                available: isAvailable
              }
            }"#,
    );
    let query_plan = build_query_plan("fixture/tests/testing.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "store") {
          {
            allProducts: products {
              __typename
              id
            }
          }
        },
        Flatten(path: "allProducts") {
          Fetch(service: "info") {
              ... on Product {
                __typename
                id
              }
            } =>
            {
              ... on Product {
                available: isAvailable
                uuid
              }
            }
          },
        },
        Flatten(path: "allProducts") {
          Fetch(service: "cost") {
              ... on Product {
                __typename
                uuid
              }
            } =>
            {
              ... on Product {
                price {
                  pricing: amount
                  currency
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
