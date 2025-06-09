use crate::{
    tests::testkit::{build_query_plan, init_logger},
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
    let query_plan = build_query_plan("fixture/tests/shared-root.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Parallel {
        Fetch(service: "name") {
          {
            product {
              name {
                model
                brand
                id
              }
            }
          }
        },
        Fetch(service: "category") {
          {
            product {
              category {
                name
                id
              }
              id
            }
          }
        },
        Fetch(service: "price") {
          {
            product {
              price {
                currency
                amount
                id
              }
            }
          }
        },
      },
    },
    "#);
    insta::assert_snapshot!(format!("{}", serde_json::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Parallel",
        "nodes": [
          {
            "kind": "Fetch",
            "serviceName": "name",
            "operationKind": "query",
            "operation": "{product{name{model brand id}}}"
          },
          {
            "kind": "Fetch",
            "serviceName": "category",
            "operationKind": "query",
            "operation": "{product{category{name id} id}}"
          },
          {
            "kind": "Fetch",
            "serviceName": "price",
            "operationKind": "query",
            "operation": "{product{price{currency amount id}}}"
          }
        ]
      }
    }
    "#);
    Ok(())
}
