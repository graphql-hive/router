use crate::{
    ast::normalization::normalize_operation,
    planner::{
        fetch::fetch_graph::build_fetch_graph_from_query_tree,
        query_plan::build_query_plan_from_fetch_graph,
        tree::{paths_to_trees, query_tree::QueryTree},
        walker::walk_operation,
    },
    tests::testkit::{init_logger, read_supergraph},
    utils::parsing::parse_operation,
};
use std::error::Error;

#[test]
fn shared_root() -> Result<(), Box<dyn Error>> {
    init_logger();
    let (graph, consumer_schema) = read_supergraph("fixture/tests/shared-root.supergraph.graphql");
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
    let document = normalize_operation(&consumer_schema, &document, None);
    let operation = document.executable_operation();
    let best_paths_per_leaf = walk_operation(&graph, operation)?;
    assert_eq!(best_paths_per_leaf.len(), 9);
    let qtps = paths_to_trees(&graph, &best_paths_per_leaf)?;
    let query_tree = QueryTree::merge_trees(qtps);
    let fetch_graph = build_fetch_graph_from_query_tree(&graph, query_tree)?;
    let query_plan = build_query_plan_from_fetch_graph(fetch_graph)?;

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
