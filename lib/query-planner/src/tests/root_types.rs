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
        Fetch(service: "price") {
          {
            product {
              price {
                id
                amount
                currency
              }
            }
          }
        },
        Fetch(service: "category") {
          {
            product {
              category {
                id
                name
              }
              id
            }
          }
        },
        Fetch(service: "name") {
          {
            product {
              name {
                id
                brand
                model
              }
            }
          }
        },
      },
    },
    "#);
    insta::assert_snapshot!(format!("{}", sonic_rs::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Parallel",
        "nodes": [
          {
            "kind": "Fetch",
            "id": 4,
            "serviceName": "price",
            "operationKind": "query",
            "operation": "{product{price{id amount currency}}}"
          },
          {
            "kind": "Fetch",
            "id": 3,
            "serviceName": "category",
            "operationKind": "query",
            "operation": "{product{category{id name} id}}"
          },
          {
            "kind": "Fetch",
            "id": 2,
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
