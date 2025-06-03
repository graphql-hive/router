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

// TODO: add a test that involves an entity call to fetch non-local fields from Book and Movie
//       to test how `... on X` affects the FetchGraph and FlattenNode.

#[test]
fn union_member_resolvable() -> Result<(), Box<dyn Error>> {
    init_logger();
    let graph = read_supergraph("fixture/tests/union-intersection.supergraph.graphql");

    let document = parse_operation(
        r#"
        query {
          media {
            ... on Book {
              title
            }
          }
        }
        "#,
    );
    let document = prepare_document(&document, None);
    let operation = document.executable_operation().unwrap();
    let best_paths_per_leaf = walk_operation(&graph, operation)?;
    let qtps = paths_to_trees(&graph, &best_paths_per_leaf)?;
    let query_tree = QueryTree::merge_trees(qtps);

    let fetch_graph = build_fetch_graph_from_query_tree(&graph, query_tree)?;
    let query_plan = build_query_plan_from_fetch_graph(fetch_graph)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "a") {
        {
          media {
            __typename
            ... on Book {
              title
            }
          }
        }
      },
    },
    "#);

    println!("{}", graph);

    Ok(())
}

#[test]
fn union_member_unresolvable() -> Result<(), Box<dyn Error>> {
    init_logger();
    let graph = read_supergraph("fixture/tests/union-intersection.supergraph.graphql");

    let document = parse_operation(
        r#"
        query {
          media {
            ... on Movie {
              title
            }
          }
        }
        "#,
    );
    let document = prepare_document(&document, None);
    let operation = document.executable_operation().unwrap();
    let best_paths_per_leaf = walk_operation(&graph, operation)?;
    let qtps = paths_to_trees(&graph, &best_paths_per_leaf)?;
    let query_tree = QueryTree::merge_trees(qtps);

    let fetch_graph = build_fetch_graph_from_query_tree(&graph, query_tree)?;
    let query_plan = build_query_plan_from_fetch_graph(fetch_graph)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "a") {
        {
          media {
            __typename
          }
        }
      },
    },
    "#);

    Ok(())
}

#[test]
fn union_member_mix() -> Result<(), Box<dyn Error>> {
    init_logger();
    let graph = read_supergraph("fixture/tests/union-intersection.supergraph.graphql");

    let document = parse_operation(
        r#"
        query {
          media {
            __typename
            ... on Book {
              title
            }
          }
        }
        "#,
    );
    let document = prepare_document(&document, None);
    let operation = document.executable_operation().unwrap();
    let best_paths_per_leaf = walk_operation(&graph, operation)?;
    let qtps = paths_to_trees(&graph, &best_paths_per_leaf)?;
    let query_tree = QueryTree::merge_trees(qtps);

    let fetch_graph = build_fetch_graph_from_query_tree(&graph, query_tree)?;
    let query_plan = build_query_plan_from_fetch_graph(fetch_graph)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "a") {
        {
          media {
            __typename
            ... on Book {
              title
            }
          }
        }
      },
    },
    "#);

    println!("{}", graph);

    Ok(())
}
