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
                amount
                currency
                id
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
                brand
                id
                model
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
            "serviceName": "price",
            "operationKind": "query",
            "operation": "query{product{price{amount currency id}}}"
          },
          {
            "kind": "Fetch",
            "serviceName": "category",
            "operationKind": "query",
            "operation": "query{product{category{id name} id}}"
          },
          {
            "kind": "Fetch",
            "serviceName": "name",
            "operationKind": "query",
            "operation": "query{product{name{brand id model}}}"
          }
        ]
      }
    }
    "#);
    Ok(())
}
