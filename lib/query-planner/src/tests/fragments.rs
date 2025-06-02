use crate::{
    planner::{
        fetch::fetch_graph::build_fetch_graph_from_query_tree,
        query_plan::build_query_plan_from_fetch_graph,
        tree::{paths_to_trees, query_tree::QueryTree},
        walker::walk_operation,
    },
    tests::testkit::{init_logger, read_supergraph},
    utils::{operation_utils::prepare_document, parsing::parse_operation},
};
use std::error::Error;

#[test]
fn simple_inline_fragment() -> Result<(), Box<dyn Error>> {
    init_logger();
    let graph = read_supergraph("fixture/tests/testing.supergraph.graphql");
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
    let document = prepare_document(&document, None);
    let operation = document.executable_operation().unwrap();
    let best_paths_per_leaf = walk_operation(&graph, operation)?;
    assert_eq!(best_paths_per_leaf.len(), 3);
    assert_eq!(best_paths_per_leaf[0].len(), 1);
    assert_eq!(best_paths_per_leaf[1].len(), 1);
    assert_eq!(best_paths_per_leaf[2].len(), 1);

    let qtps = paths_to_trees(&graph, &best_paths_per_leaf)?;
    let query_tree = QueryTree::merge_trees(qtps);

    insta::assert_snapshot!(query_tree.pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/store)
        products of Product/store
          🧩 [
            id of ID/store
          ]
          🔑 Product/info
            isAvailable of Boolean/info
          🧩 [
            🧩 [
              id of ID/store
            ]
            🔑 Product/info
              uuid of ID/info
          ]
          🔑 Product/cost
            price of Price/cost
              currency of String/cost
              amount of Float/cost
    ");

    let fetch_graph = build_fetch_graph_from_query_tree(&graph, query_tree)?;
    let query_plan = build_query_plan_from_fetch_graph(fetch_graph)?;

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
              ... on Product {
                __typename
                uuid
              }
            } =>
            {
              ... on Product {
                price {
                  currency
                  amount
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
    let graph = read_supergraph("fixture/tests/testing.supergraph.graphql");
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
    let document = prepare_document(&document, None);
    let operation = document.executable_operation().unwrap();
    let best_paths_per_leaf = walk_operation(&graph, operation)?;
    assert_eq!(best_paths_per_leaf.len(), 3);
    assert_eq!(best_paths_per_leaf[0].len(), 1);
    assert_eq!(best_paths_per_leaf[1].len(), 1);
    assert_eq!(best_paths_per_leaf[2].len(), 1);

    let qtps = paths_to_trees(&graph, &best_paths_per_leaf)?;
    let query_tree = QueryTree::merge_trees(qtps);

    insta::assert_snapshot!(query_tree.pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/store)
        products of Product/store
          🧩 [
            id of ID/store
          ]
          🔑 Product/info
            isAvailable of Boolean/info
          🧩 [
            🧩 [
              id of ID/store
            ]
            🔑 Product/info
              uuid of ID/info
          ]
          🔑 Product/cost
            price of Price/cost
              currency of String/cost
              amount of Float/cost
    ");

    let fetch_graph = build_fetch_graph_from_query_tree(&graph, query_tree)?;
    let query_plan = build_query_plan_from_fetch_graph(fetch_graph)?;

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
              ... on Product {
                __typename
                uuid
              }
            } =>
            {
              ... on Product {
                price {
                  currency
                  amount
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
