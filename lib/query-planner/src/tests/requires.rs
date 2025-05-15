use crate::{
    parse_operation,
    planner::walker::walk_operation,
    tests::testkit::{init_logger, paths_to_trees, read_supergraph},
    utils::operation_utils::get_operation_to_execute,
};
use std::error::Error;

#[test]
fn simple_requires() -> Result<(), Box<dyn Error>> {
    init_logger();
    let graph = read_supergraph("fixture/tests/simple-requires.supergraph.graphql");
    let document = parse_operation(
        r#"
        query {
          products {
            shippingEstimate
          }
        }"#,
    );
    let operation = get_operation_to_execute(&document).expect("failed to locate operation");
    let best_paths_per_leaf = walk_operation(&graph, operation)?;
    assert_eq!(best_paths_per_leaf.len(), 1);
    assert_eq!(best_paths_per_leaf[0].len(), 1);

    insta::assert_snapshot!(
      best_paths_per_leaf[0][0].pretty_print(&graph),
      @"root(Query) -(products)- Query/products -(products)- Product/products -(🔑🧩{upc})- Product/inventory -(shippingEstimate🧩{price weight})- Int/inventory"
    );

    let qtps = paths_to_trees(&graph, &best_paths_per_leaf);

    insta::assert_snapshot!(qtps[0].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/products)
        products of Product/products
          🧩 [
            upc of String/products
          ]
          🔑 Product/inventory
            🧩 [
              🧩 [
                upc of String/inventory
              ]
              🔑 Product/products
                price of Int/products
                weight of Int/products
            ]
            shippingEstimate of Int/inventory
    ");

    Ok(())
}

#[test]
fn two_same_service_calls() -> Result<(), Box<dyn Error>> {
    init_logger();
    let graph = read_supergraph("fixture/tests/two-same-service-calls.supergraph.graphql");
    let document = parse_operation(
        r#"
        query {
          products {
            isExpensive
          }
        }"#,
    );
    let operation = get_operation_to_execute(&document).expect("failed to locate operation");
    let best_paths_per_leaf = walk_operation(&graph, operation)?;
    assert_eq!(best_paths_per_leaf.len(), 1);
    assert_eq!(best_paths_per_leaf[0].len(), 1);

    insta::assert_snapshot!(
      best_paths_per_leaf[0][0].pretty_print(&graph),
      @"root(Query) -(inventory)- Query/inventory -(products)- Product/inventory -(isExpensive🧩{price})- Boolean/inventory"
    );

    let qtps = paths_to_trees(&graph, &best_paths_per_leaf);

    insta::assert_snapshot!(qtps[0].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/inventory)
        products of Product/inventory
          🧩 [
            🧩 [
              upc of String/inventory
            ]
            🔑 Product/products
              price of Int/products
          ]
          isExpensive of Boolean/inventory
    ");

    Ok(())
}

#[test]
fn simplest_requires() -> Result<(), Box<dyn Error>> {
    init_logger();
    let graph = read_supergraph("fixture/tests/simplest-requires.supergraph.graphql");
    let document = parse_operation(
        r#"
        query {
          products {
            isExpensive
          }
        }"#,
    );
    let operation = get_operation_to_execute(&document).expect("failed to locate operation");
    let best_paths_per_leaf = walk_operation(&graph, operation)?;
    assert_eq!(best_paths_per_leaf.len(), 1);
    assert_eq!(best_paths_per_leaf[0].len(), 1);

    insta::assert_snapshot!(
      best_paths_per_leaf[0][0].pretty_print(&graph),
      @"root(Query) -(products)- Query/products -(products)- Product/products -(🔑🧩{upc})- Product/inventory -(isExpensive🧩{price})- Boolean/inventory"
    );

    let qtps = paths_to_trees(&graph, &best_paths_per_leaf);

    insta::assert_snapshot!(qtps[0].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/products)
        products of Product/products
          🧩 [
            upc of String/products
          ]
          🔑 Product/inventory
            🧩 [
              🧩 [
                upc of String/inventory
              ]
              🔑 Product/products
                price of Int/products
            ]
            isExpensive of Boolean/inventory
    ");

    Ok(())
}

#[test]
fn keys_mashup() -> Result<(), Box<dyn Error>> {
    init_logger();
    let graph = read_supergraph("fixture/tests/keys-mashup.supergraph.graphql");
    let document = parse_operation(
        r#"
        query {
          b {
            id
            a {
              id
              name
              nameInB
            }
          }
        }"#,
    );
    let operation = get_operation_to_execute(&document).expect("failed to locate operation");
    let best_paths_per_leaf = walk_operation(&graph, operation)?;
    assert_eq!(best_paths_per_leaf.len(), 4);
    assert_eq!(best_paths_per_leaf[0].len(), 1);
    assert_eq!(best_paths_per_leaf[1].len(), 2);
    assert_eq!(best_paths_per_leaf[2].len(), 1);
    assert_eq!(best_paths_per_leaf[3].len(), 1);

    insta::assert_snapshot!(
      best_paths_per_leaf[0][0].pretty_print(&graph),
      @"root(Query) -(b)- Query/b -(b)- B/b -(a)- A/b -(nameInB🧩{name})- String/b"
    );
    insta::assert_snapshot!(
      best_paths_per_leaf[1][0].pretty_print(&graph),
      @"root(Query) -(b)- Query/b -(b)- B/b -(a)- A/b -(🔑🧩{pId})- A/a -(name)- String/a"
    );
    insta::assert_snapshot!(
      best_paths_per_leaf[2][0].pretty_print(&graph),
      @"root(Query) -(b)- Query/b -(b)- B/b -(a)- A/b -(id)- ID/b"
    );
    insta::assert_snapshot!(
      best_paths_per_leaf[3][0].pretty_print(&graph),
      @"root(Query) -(b)- Query/b -(b)- B/b -(id)- ID/b"
    );

    let qtps = paths_to_trees(&graph, &best_paths_per_leaf);

    insta::assert_snapshot!(qtps[0].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/b)
        b of B/b
          a of A/b
            🧩 [
              🧩 [
                pId of ID/b
              ]
              🔑 A/a
                name of String/a
              🧩 [
                id of ID/b
              ]
              🔑 A/a
                name of String/a
            ]
            nameInB of String/b
    ");

    insta::assert_snapshot!(qtps[1].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/b)
        b of B/b
          a of A/b
            🧩 [
              pId of ID/b
            ]
            🔑 A/a
              name of String/a
    ");

    insta::assert_snapshot!(qtps[2].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/b)
        b of B/b
          a of A/b
            id of ID/b
    ");

    insta::assert_snapshot!(qtps[3].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/b)
        b of B/b
          id of ID/b
    ");

    Ok(())
}
