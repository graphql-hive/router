use crate::{
    parse_operation,
    planner::{
        fetch::fetch_graph::build_fetch_graph_from_query_tree, tree::query_tree::QueryTree,
        walker::walk_operation,
    },
    tests::testkit::{init_logger, paths_to_trees, read_supergraph},
    utils::operation_utils::get_operation_to_execute,
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

    let qtps = paths_to_trees(&graph, &best_paths_per_leaf);
    insta::assert_snapshot!(qtps[0].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/b)
        feed of Post/b
          createdAt of String/b
    ");

    let query_tree = QueryTree::merge_trees(qtps);
    let fetch_graph = build_fetch_graph_from_query_tree(&graph, query_tree)?;

    insta::assert_snapshot!(format!("{}", fetch_graph), @r"
    Nodes:
    [1] Query/b {} → {feed{createdAt}} at $.

    Tree:
    [1]
    ");

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

    let qtps = paths_to_trees(&graph, &best_paths_per_leaf);
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

    // TODO: [3] should be "[b] Post {__typename id} → {createdAt} at $.aFeed.@"
    // `id` is missing now.
    insta::assert_snapshot!(format!("{}", fetch_graph), @r"
    Nodes:
    [1] Query/b {} → {bFeed{createdAt}} at $.
    [2] Query/a {} → {aFeed{id}} at $.
    [3] Post/b {__typename id} → {createdAt} at $.aFeed.@

    Tree:
    [1]
    [2]
      [3]
    ");

    Ok(())
}
