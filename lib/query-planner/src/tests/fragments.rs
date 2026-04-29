use crate::{
    tests::testkit::{build_query_plan, init_logger},
    utils::parsing::parse_operation,
};
use std::error::Error;

/// Regression test: multiple inline fragments on the same concrete type inside an abstract type
/// fragment should all be evaluated, not just the first one.
/// Uses `account(id:)` (concrete parent type `Account`) with a `... on Node` abstract fragment
/// to force the `expand_abstract_fragment` path, where two `... on Account` inline fragments
/// must both be collected and merged so all their fields appear in the query plan.
#[test]
fn multiple_inline_fragments_on_same_concrete_type_within_interface_fragment(
) -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          account(id: "a1") {
            ... on Node {
              ... on Account {
                id
              }
              ... on Account {
                username
              }
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan(
        "fixture/tests/corrupted-supergraph-node-id.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "a") {
        {
          account(id: "a1") {
            id
            username
          }
        }
      },
    },
    "#);

    Ok(())
}

#[test]
fn simple_inline_fragment() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
            query {
              products {
                price {
                  amount
                  currency
                }
                ... on Product {
                  isAvailable
                }
              }
            }"#,
    );
    let query_plan = build_query_plan("fixture/tests/testing.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "store") {
          {
            products {
              __typename
              id
            }
          }
        },
        Flatten(path: "products") {
          Fetch(service: "info") {
            {
              ... on Product {
                __typename
                id
              }
            } =>
            {
              ... on Product {
                isAvailable
                uuid
              }
            }
          },
        },
        Flatten(path: "products") {
          Fetch(service: "cost") {
            {
              ... on Product {
                __typename
                uuid
              }
            } =>
            {
              ... on Product {
                price {
                  amount
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

#[test]
fn fragment_spread() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        fragment ProductInfo on Product {
          isAvailable
        }

        query {
          products {
            price {
              amount
              currency
            }
            ...ProductInfo
          }
        }"#,
    );
    let query_plan = build_query_plan("fixture/tests/testing.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "store") {
          {
            products {
              __typename
              id
            }
          }
        },
        Flatten(path: "products") {
          Fetch(service: "info") {
            {
              ... on Product {
                __typename
                id
              }
            } =>
            {
              ... on Product {
                isAvailable
                uuid
              }
            }
          },
        },
        Flatten(path: "products") {
          Fetch(service: "cost") {
            {
              ... on Product {
                __typename
                uuid
              }
            } =>
            {
              ... on Product {
                price {
                  amount
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
