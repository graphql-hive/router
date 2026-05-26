use crate::graph::edge::PlannerOverrideContext;
use crate::{
    planner::operation_name::SubgraphOperationNameConfig,
    tests::testkit::{
        build_query_plan, build_query_plan_with_context_and_operation_names, init_logger,
    },
    utils::parsing::parse_operation,
};
use std::collections::BTreeMap;
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

#[test]
fn forwarded_operation_names_include_fetch_step_id_when_enabled() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query GetProduct {
          product {
            id
            price {
              id
              amount
            }
          }
        }"#,
    );
    let operation_name_config =
        SubgraphOperationNameConfig::new(false, BTreeMap::from([("price".to_string(), true)]));
    let query_plan = build_query_plan_with_context_and_operation_names(
        "fixture/tests/shared-root.supergraph.graphql",
        document,
        PlannerOverrideContext::default(),
        &operation_name_config,
    )?;
    let json = sonic_rs::to_string_pretty(&query_plan).unwrap_or_default();

    assert!(json.contains(r#""operationName": "GetProduct_"#));
    assert!(json.contains(r#""operation": "query GetProduct_"#));
    assert!(!json.contains(r#""operationName": "GetProduct_1""#));

    Ok(())
}
