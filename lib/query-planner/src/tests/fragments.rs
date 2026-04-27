use crate::{
    tests::testkit::{build_query_plan, init_logger},
    utils::parsing::parse_operation,
};
use std::error::Error;

/// Regression test: multiple inline fragments on the same concrete type inside an abstract type
/// fragment should all be evaluated, not just the first one.
/// When querying a field returning an interface type and using a fragment on that interface
/// containing multiple fragments on the same concrete type, all fragment fields must be included.
#[test]
fn multiple_inline_fragments_on_same_concrete_type_within_interface_fragment(
) -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          node(id: "a1") {
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
          node(id: "a1") {
            __typename
            ... on Account {
              id
              username
            }
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
