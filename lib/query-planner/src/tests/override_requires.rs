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
fn override_with_requires_many() -> Result<(), Box<dyn Error>> {
    init_logger();
    let graph = read_supergraph("fixture/tests/override_requires.supergraph.graphql");
    let document = parse_operation(
        r#"
        query {
          userInA {
            id
            name
            aName
            cName
          }
          userInB {
            id
            name
            aName
            cName
          }
          userInC {
            id
            name
            aName
            cName
          }
        }"#,
    );
    let operation = get_operation_to_execute(&document).expect("failed to locate operation");
    let best_paths_per_leaf = walk_operation(&graph, operation)?;
    assert_eq!(best_paths_per_leaf.len(), 12);

    insta::assert_snapshot!(best_paths_per_leaf[0][0].pretty_print(&graph), @"root(Query) -(c)- Query/c -(userInC)- User/c -(cName🧩{name})- String/c");
    insta::assert_snapshot!(best_paths_per_leaf[1][0].pretty_print(&graph), @"root(Query) -(c)- Query/c -(userInC)- User/c -(🔑🧩{id})- User/a -(aName🧩{name})- String/a");
    insta::assert_snapshot!(best_paths_per_leaf[2][0].pretty_print(&graph), @"root(Query) -(c)- Query/c -(userInC)- User/c -(🔑🧩{id})- User/b -(name)- String/b");
    insta::assert_snapshot!(best_paths_per_leaf[3][0].pretty_print(&graph), @"root(Query) -(c)- Query/c -(userInC)- User/c -(id)- ID/c");

    insta::assert_snapshot!(best_paths_per_leaf[4][0].pretty_print(&graph), @"root(Query) -(b)- Query/b -(userInB)- User/b -(🔑🧩{id})- User/c -(cName🧩{name})- String/c");
    insta::assert_snapshot!(best_paths_per_leaf[5][0].pretty_print(&graph), @"root(Query) -(b)- Query/b -(userInB)- User/b -(🔑🧩{id})- User/a -(aName🧩{name})- String/a");
    insta::assert_snapshot!(best_paths_per_leaf[6][0].pretty_print(&graph), @"root(Query) -(b)- Query/b -(userInB)- User/b -(name)- String/b");
    insta::assert_snapshot!(best_paths_per_leaf[7][0].pretty_print(&graph), @"root(Query) -(b)- Query/b -(userInB)- User/b -(id)- ID/b");

    insta::assert_snapshot!(best_paths_per_leaf[8][0].pretty_print(&graph), @"root(Query) -(a)- Query/a -(userInA)- User/a -(🔑🧩{id})- User/c -(cName🧩{name})- String/c");
    insta::assert_snapshot!(best_paths_per_leaf[9][0].pretty_print(&graph), @"root(Query) -(a)- Query/a -(userInA)- User/a -(aName🧩{name})- String/a");
    insta::assert_snapshot!(best_paths_per_leaf[10][0].pretty_print(&graph), @"root(Query) -(a)- Query/a -(userInA)- User/a -(🔑🧩{id})- User/b -(name)- String/b");
    insta::assert_snapshot!(best_paths_per_leaf[11][0].pretty_print(&graph), @"root(Query) -(a)- Query/a -(userInA)- User/a -(id)- ID/a");

    let qtps = paths_to_trees(&graph, &best_paths_per_leaf);

    let query_tree = QueryTree::merge_trees(qtps);
    insta::assert_snapshot!(query_tree.pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/c)
        userInC of User/c
          🧩 [
            🧩 [
              id of ID/c
            ]
            🔑 User/b
              name of String/b
          ]
          cName of String/c
          🧩 [
            id of ID/c
          ]
          🔑 User/a
            🧩 [
              🧩 [
                id of ID/a
              ]
              🔑 User/b
                name of String/b
            ]
            aName of String/a
          🧩 [
            id of ID/c
          ]
          🔑 User/b
            name of String/b
          id of ID/c
      🚪 (Query/b)
        userInB of User/b
          🧩 [
            id of ID/b
          ]
          🔑 User/c
            🧩 [
              🧩 [
                id of ID/c
              ]
              🔑 User/b
                name of String/b
            ]
            cName of String/c
          🧩 [
            id of ID/b
          ]
          🔑 User/a
            🧩 [
              🧩 [
                id of ID/a
              ]
              🔑 User/b
                name of String/b
            ]
            aName of String/a
          name of String/b
          id of ID/b
      🚪 (Query/a)
        userInA of User/a
          🧩 [
            id of ID/a
          ]
          🔑 User/c
            🧩 [
              🧩 [
                id of ID/c
              ]
              🔑 User/b
                name of String/b
            ]
            cName of String/c
          🧩 [
            🧩 [
              id of ID/a
            ]
            🔑 User/b
              name of String/b
          ]
          aName of String/a
          🧩 [
            id of ID/a
          ]
          🔑 User/b
            name of String/b
          id of ID/a
    ");

    // TODO: make sure this one works, by adding requires support.
    // let fetch_graph = build_fetch_graph_from_query_tree(&graph, query_tree)?;
    // insta::assert_snapshot!(format!("{}", fetch_graph), @r"...");

    Ok(())
}

#[test]
fn override_with_requires_cname_in_c() -> Result<(), Box<dyn Error>> {
    init_logger();
    let graph = read_supergraph("fixture/tests/override_requires.supergraph.graphql");
    let document = parse_operation(
        r#"
        query {
          userInC {
            cName
          }
        }"#,
    );
    let operation = get_operation_to_execute(&document).expect("failed to locate operation");
    let best_paths_per_leaf = walk_operation(&graph, operation)?;
    assert_eq!(best_paths_per_leaf.len(), 1);
    let qtps = paths_to_trees(&graph, &best_paths_per_leaf);

    let query_tree = QueryTree::merge_trees(qtps);
    insta::assert_snapshot!(query_tree.pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/c)
        userInC of User/c
          🧩 [
            🧩 [
              id of ID/c
            ]
            🔑 User/b
              name of String/b
          ]
          cName of String/c
    ");

    // TODO: make sure this one works, by adding requires support.
    // let fetch_graph = build_fetch_graph_from_query_tree(&graph, query_tree)?;
    // insta::assert_snapshot!(format!("{}", fetch_graph), @r"...");

    Ok(())
}

#[test]
fn override_with_requires_cname_in_a() -> Result<(), Box<dyn Error>> {
    init_logger();
    let graph = read_supergraph("fixture/tests/override_requires.supergraph.graphql");
    let document = parse_operation(
        r#"
        query {
          userInA {
            cName
          }
        }"#,
    );
    let operation = get_operation_to_execute(&document).expect("failed to locate operation");
    let best_paths_per_leaf = walk_operation(&graph, operation)?;
    assert_eq!(best_paths_per_leaf.len(), 1);
    let qtps = paths_to_trees(&graph, &best_paths_per_leaf);

    let query_tree = QueryTree::merge_trees(qtps);
    insta::assert_snapshot!(query_tree.pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/a)
        userInA of User/a
          🧩 [
            id of ID/a
          ]
          🔑 User/c
            🧩 [
              🧩 [
                id of ID/c
              ]
              🔑 User/b
                name of String/b
            ]
            cName of String/c
    ");

    // TODO: make sure this one works, by adding requires support.
    // let fetch_graph = build_fetch_graph_from_query_tree(&graph, query_tree)?;
    // insta::assert_snapshot!(format!("{}", fetch_graph), @r"...");

    Ok(())
}

#[test]
fn override_with_requires_aname_in_a() -> Result<(), Box<dyn Error>> {
    init_logger();
    let graph = read_supergraph("fixture/tests/override_requires.supergraph.graphql");
    let document = parse_operation(
        r#"
        query {
          userInA {
            aName
          }
        }"#,
    );
    let operation = get_operation_to_execute(&document).expect("failed to locate operation");
    let best_paths_per_leaf = walk_operation(&graph, operation)?;
    assert_eq!(best_paths_per_leaf.len(), 1);
    let qtps = paths_to_trees(&graph, &best_paths_per_leaf);

    let query_tree = QueryTree::merge_trees(qtps);
    insta::assert_snapshot!(query_tree.pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/a)
        userInA of User/a
          🧩 [
            🧩 [
              id of ID/a
            ]
            🔑 User/b
              name of String/b
          ]
          aName of String/a
    ");

    // TODO: make sure this one works, by adding requires support.
    // let fetch_graph = build_fetch_graph_from_query_tree(&graph, query_tree)?;
    // insta::assert_snapshot!(format!("{}", fetch_graph), @r"...");

    Ok(())
}
