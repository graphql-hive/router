use crate::{
    parse_operation,
    planner::{tree::query_tree::QueryTree, walker::walk_operation},
    tests::testkit::{init_logger, paths_to_trees, read_supergraph},
    utils::operation_utils::get_operation_to_execute,
};
use std::error::Error;

#[test]
fn testing() -> Result<(), Box<dyn Error>> {
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
                isAvailable
              }
            }"#,
    );
    let operation = get_operation_to_execute(&document).expect("failed to locate operation");
    let best_paths_per_leaf = walk_operation(&graph, operation)?;
    assert_eq!(best_paths_per_leaf.len(), 3);
    assert_eq!(best_paths_per_leaf[0].len(), 1);
    assert_eq!(best_paths_per_leaf[1].len(), 1);
    assert_eq!(best_paths_per_leaf[2].len(), 1);

    insta::assert_snapshot!(
      best_paths_per_leaf[0][0].pretty_print(&graph),
      @"root(Query) -(STORE)- Query/STORE -(products)- Product/STORE -(🔑🧩{id})- Product/INFO -(isAvailable)- Boolean/INFO"
    );
    insta::assert_snapshot!(
      best_paths_per_leaf[1][0].pretty_print(&graph),
      @"root(Query) -(STORE)- Query/STORE -(products)- Product/STORE -(🔑🧩{uuid})- Product/COST -(price)- Price/COST -(currency)- String/COST"
    );
    insta::assert_snapshot!(
      best_paths_per_leaf[2][0].pretty_print(&graph),
      @"root(Query) -(STORE)- Query/STORE -(products)- Product/STORE -(🔑🧩{uuid})- Product/COST -(price)- Price/COST -(amount)- Float/COST"
    );

    let qtps = paths_to_trees(&graph, &best_paths_per_leaf);

    insta::assert_snapshot!(qtps[0].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/STORE)
        products of Product/STORE
          🧩 [
            id of ID/STORE
          ]
          🔑 Product/INFO
            isAvailable of Boolean/INFO
    ");

    insta::assert_snapshot!(qtps[1].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/STORE)
        products of Product/STORE
          🧩 [
            🧩 [
              id of ID/STORE
            ]
            🔑 Product/INFO
              uuid of ID/INFO
          ]
          🔑 Product/COST
            price of Price/COST
              currency of String/COST
    ");

    insta::assert_snapshot!(qtps[2].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/STORE)
        products of Product/STORE
          🧩 [
            🧩 [
              id of ID/STORE
            ]
            🔑 Product/INFO
              uuid of ID/INFO
          ]
          🔑 Product/COST
            price of Price/COST
              amount of Float/COST
    ");

    let gqt = QueryTree::merge_trees(qtps);

    insta::assert_snapshot!(gqt.pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/STORE)
        products of Product/STORE
          🧩 [
            id of ID/STORE
          ]
          🔑 Product/INFO
            isAvailable of Boolean/INFO
          🧩 [
            🧩 [
              id of ID/STORE
            ]
            🔑 Product/INFO
              uuid of ID/INFO
          ]
          🔑 Product/COST
            price of Price/COST
              currency of String/COST
              amount of Float/COST
    ");

    Ok(())
}

#[test]
fn parent_entity_call() -> Result<(), Box<dyn Error>> {
    init_logger();
    let graph = read_supergraph("fixture/tests/parent-entity-call.supergraph.graphql");
    let document = parse_operation(
        r#"
        query {
          products {
            category {
              details {
                products
              }
            }
          }
        }"#,
    );
    let operation = get_operation_to_execute(&document).expect("failed to locate operation");
    let best_paths_per_leaf = walk_operation(&graph, operation)?;
    assert_eq!(best_paths_per_leaf.len(), 1);
    assert_eq!(best_paths_per_leaf[0].len(), 1);

    insta::assert_snapshot!(
      best_paths_per_leaf[0][0].pretty_print(&graph),
      @"root(Query) -(A)- Query/A -(products)- Product/A -(🔑🧩{id pid})- Product/C -(category)- Category/C -(details)- CategoryDetails/C -(products)- Int/C"
    );

    let qtps = paths_to_trees(&graph, &best_paths_per_leaf);

    insta::assert_snapshot!(qtps[0].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/A)
        products of Product/A
          🧩 [
            id of ID/A
            pid of ID/A
          ]
          🔑 Product/C
            category of Category/C
              details of CategoryDetails/C
                products of Int/C
    ");

    Ok(())
}

#[test]
fn parent_entity_call_complex() -> Result<(), Box<dyn Error>> {
    init_logger();
    let graph = read_supergraph("fixture/tests/parent-entity-call-complex.supergraph.graphql");
    let document = parse_operation(
        r#"
        query {
          productFromD(id: "1") {
            id
            name
            category {
              id
              name
              details
            }
          }
        }"#,
    );
    let operation = get_operation_to_execute(&document).expect("failed to locate operation");
    let best_paths_per_leaf = walk_operation(&graph, operation)?;
    assert_eq!(best_paths_per_leaf.len(), 5);
    assert_eq!(best_paths_per_leaf[0].len(), 1);
    assert_eq!(best_paths_per_leaf[1].len(), 1);
    assert_eq!(best_paths_per_leaf[2].len(), 1);
    assert_eq!(best_paths_per_leaf[3].len(), 1);
    assert_eq!(best_paths_per_leaf[4].len(), 1);

    insta::assert_snapshot!(best_paths_per_leaf[0][0].pretty_print(&graph), @"root(Query) -(D)- Query/D -(productFromD)- Product/D -(🔑🧩{id})- Product/A -(category)- Category/A -(details)- String/A");
    insta::assert_snapshot!(best_paths_per_leaf[1][0].pretty_print(&graph), @"root(Query) -(D)- Query/D -(productFromD)- Product/D -(🔑🧩{id})- Product/B -(category)- Category/B -(🔑🧩{id})- Category/C -(name)- String/C");
    insta::assert_snapshot!(best_paths_per_leaf[2][0].pretty_print(&graph), @"root(Query) -(D)- Query/D -(productFromD)- Product/D -(🔑🧩{id})- Product/B -(category)- Category/B -(id)- ID/B");
    insta::assert_snapshot!(best_paths_per_leaf[3][0].pretty_print(&graph), @"root(Query) -(D)- Query/D -(productFromD)- Product/D -(name)- String/D");
    insta::assert_snapshot!(best_paths_per_leaf[4][0].pretty_print(&graph), @"root(Query) -(D)- Query/D -(productFromD)- Product/D -(id)- ID/D");

    let qtps = paths_to_trees(&graph, &best_paths_per_leaf);

    insta::assert_snapshot!(qtps[0].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/D)
        productFromD of Product/D
          🧩 [
            id of ID/D
          ]
          🔑 Product/A
            category of Category/A
              details of String/A
    ");

    insta::assert_snapshot!(qtps[1].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/D)
        productFromD of Product/D
          🧩 [
            id of ID/D
          ]
          🔑 Product/B
            category of Category/B
              🧩 [
                id of ID/B
              ]
              🔑 Category/C
                name of String/C
    ");
    insta::assert_snapshot!(qtps[2].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/D)
        productFromD of Product/D
          🧩 [
            id of ID/D
          ]
          🔑 Product/B
            category of Category/B
              id of ID/B
    ");
    insta::assert_snapshot!(qtps[3].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/D)
        productFromD of Product/D
          name of String/D
    ");
    insta::assert_snapshot!(qtps[4].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/D)
        productFromD of Product/D
          id of ID/D
    ");

    Ok(())
}

#[test]
fn complex_entity_call() -> Result<(), Box<dyn Error>> {
    init_logger();
    let graph = read_supergraph("fixture/tests/complex-entity-call.supergraph.graphql");
    let document = parse_operation(
        r#"
        {
          topProducts {
            products {
              id
              price {
                price
              }
            }
          }
        }"#,
    );
    let operation = get_operation_to_execute(&document).expect("failed to locate operation");
    let best_paths_per_leaf = walk_operation(&graph, operation)?;
    assert_eq!(best_paths_per_leaf.len(), 2);
    assert_eq!(best_paths_per_leaf[0].len(), 1);
    assert_eq!(best_paths_per_leaf[1].len(), 1);

    insta::assert_snapshot!(best_paths_per_leaf[0][0].pretty_print(&graph), @"root(Query) -(PRODUCTS)- Query/PRODUCTS -(topProducts)- ProductList/PRODUCTS -(products)- Product/PRODUCTS -(🔑🧩{id pid category{id tag}})- Product/PRICE -(price)- Price/PRICE -(price)- Float/PRICE");
    insta::assert_snapshot!(best_paths_per_leaf[1][0].pretty_print(&graph), @"root(Query) -(PRODUCTS)- Query/PRODUCTS -(topProducts)- ProductList/PRODUCTS -(products)- Product/PRODUCTS -(id)- String/PRODUCTS");

    let qtps = paths_to_trees(&graph, &best_paths_per_leaf);

    // TODO: Understand why "tag" and "id" are not both under the same "category of Category/PRODUCTS"
    insta::assert_snapshot!(qtps[0].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/PRODUCTS)
        topProducts of ProductList/PRODUCTS
          products of Product/PRODUCTS
            🧩 [
              id of String/PRODUCTS
              🧩 [
                id of String/PRODUCTS
              ]
              🔑 Product/LINK
                pid of String/LINK
              category of Category/PRODUCTS
                tag of String/PRODUCTS
                id of String/PRODUCTS
            ]
            🔑 Product/PRICE
              price of Price/PRICE
                price of Float/PRICE
    ");

    insta::assert_snapshot!(qtps[1].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/PRODUCTS)
        topProducts of ProductList/PRODUCTS
          products of Product/PRODUCTS
            id of String/PRODUCTS
    ");

    Ok(())
}
