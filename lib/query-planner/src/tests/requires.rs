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
      @"root(Query) -(PRODUCTS)- Query/PRODUCTS -(products)- Product/PRODUCTS -(🔑🧩{upc})- Product/INVENTORY -(shippingEstimate🧩{price weight})- Int/INVENTORY"
    );

    let qtps = paths_to_trees(&graph, &best_paths_per_leaf);

    insta::assert_snapshot!(qtps[0].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/PRODUCTS)
        products of Product/PRODUCTS
          🧩 [
            upc of String/PRODUCTS
          ]
          🔑 Product/INVENTORY
            🧩 [
              🧩 [
                upc of String/INVENTORY
              ]
              🔑 Product/PRODUCTS
                price of Int/PRODUCTS
              🧩 [
                upc of String/INVENTORY
              ]
              🔑 Product/PRODUCTS
                weight of Int/PRODUCTS
            ]
            shippingEstimate of Int/INVENTORY
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
      @"root(Query) -(INVENTORY)- Query/INVENTORY -(products)- Product/INVENTORY -(isExpensive🧩{price})- Boolean/INVENTORY"
    );

    let qtps = paths_to_trees(&graph, &best_paths_per_leaf);

    insta::assert_snapshot!(qtps[0].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/INVENTORY)
        products of Product/INVENTORY
          🧩 [
            🧩 [
              upc of String/INVENTORY
            ]
            🔑 Product/PRODUCTS
              price of Int/PRODUCTS
          ]
          isExpensive of Boolean/INVENTORY
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
      @"root(Query) -(PRODUCTS)- Query/PRODUCTS -(products)- Product/PRODUCTS -(🔑🧩{upc})- Product/INVENTORY -(isExpensive🧩{price})- Boolean/INVENTORY"
    );

    let qtps = paths_to_trees(&graph, &best_paths_per_leaf);

    insta::assert_snapshot!(qtps[0].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/PRODUCTS)
        products of Product/PRODUCTS
          🧩 [
            upc of String/PRODUCTS
          ]
          🔑 Product/INVENTORY
            🧩 [
              🧩 [
                upc of String/INVENTORY
              ]
              🔑 Product/PRODUCTS
                price of Int/PRODUCTS
            ]
            isExpensive of Boolean/INVENTORY
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
      @"root(Query) -(B)- Query/B -(b)- B/B -(a)- A/B -(nameInB🧩{name})- String/B"
    );
    insta::assert_snapshot!(
      best_paths_per_leaf[1][0].pretty_print(&graph),
      @"root(Query) -(B)- Query/B -(b)- B/B -(a)- A/B -(🔑🧩{pId})- A/A -(name)- String/A"
    );
    insta::assert_snapshot!(
      best_paths_per_leaf[2][0].pretty_print(&graph),
      @"root(Query) -(B)- Query/B -(b)- B/B -(a)- A/B -(id)- ID/B"
    );
    insta::assert_snapshot!(
      best_paths_per_leaf[3][0].pretty_print(&graph),
      @"root(Query) -(B)- Query/B -(b)- B/B -(id)- ID/B"
    );

    let qtps = paths_to_trees(&graph, &best_paths_per_leaf);

    insta::assert_snapshot!(qtps[0].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/B)
        b of B/B
          a of A/B
            🧩 [
              🧩 [
                pId of ID/B
              ]
              🔑 A/A
                name of String/A
              🧩 [
                id of ID/B
              ]
              🔑 A/A
                name of String/A
            ]
            nameInB of String/B
    ");

    // TODO: this is incorrect, should be:
    /*
    root
      Query of Query/b #9
        b of B/b #10
          a of A/b #12
            🧩 #21 [
              id of ID/b #13
            ]
            🔑 A/a #21
              name of String/a #5
    */
    insta::assert_snapshot!(qtps[1].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/B)
        b of B/B
          a of A/B
            🧩 [
              pId of ID/B
            ]
            🔑 A/A
              name of String/A
    ");

    insta::assert_snapshot!(qtps[2].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/B)
        b of B/B
          a of A/B
            id of ID/B
    ");

    insta::assert_snapshot!(qtps[3].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/B)
        b of B/B
          id of ID/B
    ");

    Ok(())
}
