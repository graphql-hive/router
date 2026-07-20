use crate::{
    tests::testkit::{build_query_plan_with_defaults, init_logger},
    utils::parsing::parse_operation,
};
use std::error::Error;

#[test]
fn query_on_renamed_root_query_type() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          product(id: "1") {
            name
            price
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/renamed-root-types.supergraph.graphql",
        document,
    )?;

    let json = sonic_rs::to_string_pretty(&query_plan).unwrap_or_default();

    // The root type is named RootQuery, but the operation kind should still be "query"
    assert!(
        json.contains(r#""operationKind": "query""#),
        "Expected operationKind to be 'query', got:\n{}",
        json
    );
    assert!(
        !json.contains(r#""operationKind": "mutation""#),
        "Should not contain mutation operation kind"
    );

    // Display format should not show 'mutation' keyword (it's a query)
    let display = format!("{}", query_plan);
    assert!(
        !display.contains("mutation"),
        "Display should not contain 'mutation' keyword for a query operation:\n{}",
        display
    );

    insta::assert_snapshot!(display, @r#"
    QueryPlan {
      Fetch(service: "a") {
        {
          product(id: "1") {
            name
            price
          }
        }
      },
    },
    "#);

    Ok(())
}

#[test]
fn mutation_on_renamed_root_mutation_type() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        mutation {
          addProduct(input: { name: "new", price: 599.99 }) {
            name
            price
            isExpensive
            isAvailable
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/renamed-root-types.supergraph.graphql",
        document,
    )?;

    let json = sonic_rs::to_string_pretty(&query_plan).unwrap_or_default();

    // The root type is named RootMutation, but the operation kind should still be "mutation"
    assert!(
        json.contains(r#""operationKind": "mutation""#),
        "Expected operationKind to be 'mutation', got:\n{}",
        json
    );

    // Display format should show 'mutation' keyword
    let display = format!("{}", query_plan);
    assert!(
        display.contains("mutation"),
        "Display should contain 'mutation' keyword for a mutation operation:\n{}",
        display
    );

    insta::assert_snapshot!(display, @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          mutation {
            addProduct(input: {name: "new", price: 599.99}) {
              __typename
              name
              price
              id
            }
          }
        },
        Flatten(path: "addProduct") {
          Fetch(service: "b") {
            {
              ... on Product {
                __typename
                price
                id
              }
            } =>
            {
              ... on Product {
                isExpensive
                isAvailable
              }
            }
          },
        },
      },
    },
    "#);

    Ok(())
}
