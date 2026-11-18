use crate::query_planner::{
    tests::testkit::{build_query_plan_with_defaults, init_logger},
    utils::parsing::parse_operation,
};
use std::error::Error;

#[test]
fn shared_root() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          product {
            id
            name {
              id
              brand
              model
            }
            category {
              id
              name
            }
            price {
              id
              amount
              currency
            }
          }
        }"#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/tests/shared-root.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [4] price           -
      [3] category        -
      [2] name            -

    [4] price
      out
        { product { price { id amount currency } } }

    [3] category
      out
        { product { category { id name } id } }

    [2] name
      out
        { product { name { id brand model } } }
    ");
    insta::assert_snapshot!(format!("{}", sonic_rs::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Parallel",
        "nodes": [
          {
            "kind": "Fetch",
            "serviceName": "price",
            "operationKind": "query",
            "operation": "{product{price{id amount currency}}}"
          },
          {
            "kind": "Fetch",
            "serviceName": "category",
            "operationKind": "query",
            "operation": "{product{category{id name} id}}"
          },
          {
            "kind": "Fetch",
            "serviceName": "name",
            "operationKind": "query",
            "operation": "{product{name{id brand model}}}"
          }
        ]
      }
    }
    "#);
    Ok(())
}
