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
      @"root(Query) -(PRODUCTS)- Query/PRODUCTS -(products)- Product/PRODUCTS -(🔑🧩upc)- Product/INVENTORY -(shippingEstimate 🧩{price weight})- Int/INVENTORY"
    );

    let qtps = paths_to_trees(&graph, &best_paths_per_leaf);

    // TODO: this is incorrect, should be:
    /*
    root
      Query of Query/products #8
        products of Product/products #9
          🧩 #14 [
            upc of String/products #10
          ]
          🔑 Product/inventory #14
            🧩 #6 [
              🧩 #15 [
                upc of String/inventory #2
              ]
              🔑 Product/products #15
                price of Int/products #12
                weight of Int/products #11
            ]
            shippingEstimate of Int/inventory #6
    */
    insta::assert_snapshot!(qtps[0].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/PRODUCTS)
        products of Product/PRODUCTS
          🧩 [
            upc of String/PRODUCTS
          ]
          🔑 Product/INVENTORY
            shippingEstimate 🧩{price weight} of Int/INVENTORY
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
      @"root(Query) -(INVENTORY)- Query/INVENTORY -(products)- Product/INVENTORY -(isExpensive 🧩{price})- Boolean/INVENTORY"
    );

    let qtps = paths_to_trees(&graph, &best_paths_per_leaf);

    // TODO: this is incorrect, should be:
    /*
    root
      Query of Query/inventory #1
        products of Product/inventory #2
          🧩 #5 [
            🧩 #10 [
              upc of String/inventory #3
            ]
            🔑 Product/products #10
              price of Int/products #8
          ]
          isExpensive of Boolean/inventory #5
    */
    insta::assert_snapshot!(qtps[0].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/INVENTORY)
        products of Product/INVENTORY
          isExpensive 🧩{price} of Boolean/INVENTORY
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
      @"root(Query) -(PRODUCTS)- Query/PRODUCTS -(products)- Product/PRODUCTS -(🔑🧩upc)- Product/INVENTORY -(isExpensive 🧩{price})- Boolean/INVENTORY"
    );

    let qtps = paths_to_trees(&graph, &best_paths_per_leaf);

    // TODO: this is incorrect, should be:
    /*
    root
      Query of Query/products #5
        products of Product/products #6
          🧩 #9 [
            upc of String/products #7
          ]
          🔑 Product/inventory #9
            🧩 #4 [
              🧩 #10 [
                upc of String/inventory #2
              ]
              🔑 Product/products #10
                price of Int/products #8
            ]
            isExpensive of Boolean/inventory #4
    */
    insta::assert_snapshot!(qtps[0].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/PRODUCTS)
        products of Product/PRODUCTS
          🧩 [
            upc of String/PRODUCTS
          ]
          🔑 Product/INVENTORY
            isExpensive 🧩{price} of Boolean/INVENTORY
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
      @"root(Query) -(B)- Query/B -(b)- B/B -(a)- A/B -(nameInB 🧩{name})- String/B"
    );
    insta::assert_snapshot!(
      best_paths_per_leaf[1][0].pretty_print(&graph),
      @"root(Query) -(B)- Query/B -(b)- B/B -(a)- A/B -(🔑🧩pId)- A/A -(name)- String/A"
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

    // TODO: this is incorrect, should be:
    /*
    root
      Query of Query/b #9
        b of B/b #10
          a of A/b #12
            🧩 #17 [
              🧩 #21 [
                id of ID/b #13
              ]
              🔑 A/a #21
                name of String/a #5
            ]
            nameInB of String/b #17
    */
    insta::assert_snapshot!(qtps[0].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/B)
        b of B/B
          a of A/B
            nameInB 🧩{name} of String/B
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
