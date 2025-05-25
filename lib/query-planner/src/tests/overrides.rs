use crate::{
    planner::{
        fetch::fetch_graph::build_fetch_graph_from_query_tree,
        query_plan::build_query_plan_from_fetch_graph,
        tree::{paths_to_trees, query_tree::QueryTree},
        walker::walk_operation,
    },
    tests::testkit::{init_logger, read_supergraph},
    utils::{operation_utils::get_operation_to_execute, parsing::parse_operation},
};
use std::error::Error;

#[test]
fn single_simple_overrides() -> Result<(), Box<dyn Error>> {
    init_logger();
    let graph = read_supergraph("fixture/tests/simple_overrides.supergraph.graphql");
    let document = parse_operation(
        r#"
        query {
          feed {
            createdAt
          }
        }"#,
    );
    let operation = get_operation_to_execute(&document).expect("failed to locate operation");
    let best_paths_per_leaf = walk_operation(&graph, operation)?;
    assert_eq!(best_paths_per_leaf.len(), 1);
    assert_eq!(best_paths_per_leaf[0].len(), 1);

    insta::assert_snapshot!(
      best_paths_per_leaf[0][0].pretty_print(&graph),
      @"root(Query) -(b)- Query/b -(feed)- Post/b -(createdAt)- String/b"
    );

    let qtps = paths_to_trees(&graph, &best_paths_per_leaf)?;
    insta::assert_snapshot!(qtps[0].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/b)
        feed of Post/b
          createdAt of String/b
    ");

    let query_tree = QueryTree::merge_trees(qtps);
    let fetch_graph = build_fetch_graph_from_query_tree(&graph, query_tree)?;
    let query_plan = build_query_plan_from_fetch_graph(fetch_graph)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "b") {
        {
          feed {
            createdAt
          }
        }
      },
    },
    "#);
    insta::assert_snapshot!(format!("{}", serde_json::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Fetch",
        "serviceName": "b",
        "operationKind": "query",
        "operation": "{feed{createdAt}}"
      }
    }
    "#);
    Ok(())
}

#[test]
fn two_fields_simple_overrides() -> Result<(), Box<dyn Error>> {
    init_logger();
    let graph = read_supergraph("fixture/tests/simple_overrides.supergraph.graphql");
    let document = parse_operation(
        r#"
        query {
          aFeed {
            createdAt
          }
          bFeed {
            createdAt
          }
        }"#,
    );
    let operation = get_operation_to_execute(&document).expect("failed to locate operation");
    let best_paths_per_leaf = walk_operation(&graph, operation)?;
    assert_eq!(best_paths_per_leaf.len(), 2);
    assert_eq!(best_paths_per_leaf[0].len(), 1);
    assert_eq!(best_paths_per_leaf[1].len(), 1);

    insta::assert_snapshot!(
      best_paths_per_leaf[0][0].pretty_print(&graph),
      @"root(Query) -(b)- Query/b -(bFeed)- Post/b -(createdAt)- String/b"
    );
    insta::assert_snapshot!(
      best_paths_per_leaf[1][0].pretty_print(&graph),
      @"root(Query) -(a)- Query/a -(aFeed)- Post/a -(🔑🧩{id})- Post/b -(createdAt)- String/b"
    );

    let qtps = paths_to_trees(&graph, &best_paths_per_leaf)?;
    insta::assert_snapshot!(qtps[0].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/b)
        bFeed of Post/b
          createdAt of String/b
    ");
    insta::assert_snapshot!(qtps[1].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/a)
        aFeed of Post/a
          🧩 [
            id of ID/a
          ]
          🔑 Post/b
            createdAt of String/b
    ");

    let query_tree = QueryTree::merge_trees(qtps);
    insta::assert_snapshot!(query_tree.pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/b)
        bFeed of Post/b
          createdAt of String/b
      🚪 (Query/a)
        aFeed of Post/a
          🧩 [
            id of ID/a
          ]
          🔑 Post/b
            createdAt of String/b
    ");

    let fetch_graph = build_fetch_graph_from_query_tree(&graph, query_tree)?;
    let query_plan = build_query_plan_from_fetch_graph(fetch_graph)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Parallel {
          Fetch(service: "a") {
            {
              aFeed {
                __typename
                id
              }
            }
          },
          Fetch(service: "b") {
            {
              bFeed {
                createdAt
              }
            }
          },
        },
        Flatten(path: "aFeed.@") {
          Fetch(service: "b") {
              ... on Post {
                __typename
                id
              }
            } =>
            {
              ... on Post {
                createdAt
              }
            }
          },
        },
      },
    },
    "#);
    insta::assert_snapshot!(format!("{}", serde_json::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Sequence",
        "nodes": [
          {
            "kind": "Parallel",
            "nodes": [
              {
                "kind": "Fetch",
                "serviceName": "a",
                "operationKind": "query",
                "operation": "{aFeed{__typename id}}"
              },
              {
                "kind": "Fetch",
                "serviceName": "b",
                "operationKind": "query",
                "operation": "{bFeed{createdAt}}"
              }
            ]
          },
          {
            "kind": "Flatten",
            "path": [
              "aFeed",
              "@"
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "b",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Post{createdAt}}}",
              "requires": [
                {
                  "kind": "InlineFragment",
                  "typeCondition": "Post",
                  "selections": [
                    {
                      "kind": "Field",
                      "name": "__typename"
                    },
                    {
                      "kind": "Field",
                      "name": "id"
                    }
                  ]
                }
              ]
            }
          }
        ]
      }
    }
    "#);
    Ok(())
}
