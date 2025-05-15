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
      @"root(Query) -(store)- Query/store -(products)- Product/store -(🔑🧩{id})- Product/info -(isAvailable)- Boolean/info"
    );
    insta::assert_snapshot!(
      best_paths_per_leaf[1][0].pretty_print(&graph),
      @"root(Query) -(store)- Query/store -(products)- Product/store -(🔑🧩{uuid})- Product/cost -(price)- Price/cost -(currency)- String/cost"
    );
    insta::assert_snapshot!(
      best_paths_per_leaf[2][0].pretty_print(&graph),
      @"root(Query) -(store)- Query/store -(products)- Product/store -(🔑🧩{uuid})- Product/cost -(price)- Price/cost -(amount)- Float/cost"
    );

    let qtps = paths_to_trees(&graph, &best_paths_per_leaf);

    insta::assert_snapshot!(qtps[0].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/store)
        products of Product/store
          🧩 [
            id of ID/store
          ]
          🔑 Product/info
            isAvailable of Boolean/info
    ");

    insta::assert_snapshot!(qtps[1].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/store)
        products of Product/store
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
    ");

    insta::assert_snapshot!(qtps[2].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/store)
        products of Product/store
          🧩 [
            🧩 [
              id of ID/store
            ]
            🔑 Product/info
              uuid of ID/info
          ]
          🔑 Product/cost
            price of Price/cost
              amount of Float/cost
    ");

    let gqt = QueryTree::merge_trees(qtps);

    insta::assert_snapshot!(gqt.pretty_print(&graph)?, @r"
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
      @"root(Query) -(a)- Query/a -(products)- Product/a -(🔑🧩{id pid})- Product/c -(category)- Category/c -(details)- CategoryDetails/c -(products)- Int/c"
    );

    let qtps = paths_to_trees(&graph, &best_paths_per_leaf);

    insta::assert_snapshot!(qtps[0].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/a)
        products of Product/a
          🧩 [
            id of ID/a
            pid of ID/a
          ]
          🔑 Product/c
            category of Category/c
              details of CategoryDetails/c
                products of Int/c
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

    insta::assert_snapshot!(best_paths_per_leaf[0][0].pretty_print(&graph), @"root(Query) -(d)- Query/d -(productFromD)- Product/d -(🔑🧩{id})- Product/a -(category)- Category/a -(details)- String/a");
    insta::assert_snapshot!(best_paths_per_leaf[1][0].pretty_print(&graph), @"root(Query) -(d)- Query/d -(productFromD)- Product/d -(🔑🧩{id})- Product/b -(category)- Category/b -(🔑🧩{id})- Category/c -(name)- String/c");
    insta::assert_snapshot!(best_paths_per_leaf[2][0].pretty_print(&graph), @"root(Query) -(d)- Query/d -(productFromD)- Product/d -(🔑🧩{id})- Product/b -(category)- Category/b -(id)- ID/b");
    insta::assert_snapshot!(best_paths_per_leaf[3][0].pretty_print(&graph), @"root(Query) -(d)- Query/d -(productFromD)- Product/d -(name)- String/d");
    insta::assert_snapshot!(best_paths_per_leaf[4][0].pretty_print(&graph), @"root(Query) -(d)- Query/d -(productFromD)- Product/d -(id)- ID/d");

    let qtps = paths_to_trees(&graph, &best_paths_per_leaf);

    insta::assert_snapshot!(qtps[0].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/d)
        productFromD of Product/d
          🧩 [
            id of ID/d
          ]
          🔑 Product/a
            category of Category/a
              details of String/a
    ");

    insta::assert_snapshot!(qtps[1].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/d)
        productFromD of Product/d
          🧩 [
            id of ID/d
          ]
          🔑 Product/b
            category of Category/b
              🧩 [
                id of ID/b
              ]
              🔑 Category/c
                name of String/c
    ");
    insta::assert_snapshot!(qtps[2].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/d)
        productFromD of Product/d
          🧩 [
            id of ID/d
          ]
          🔑 Product/b
            category of Category/b
              id of ID/b
    ");
    insta::assert_snapshot!(qtps[3].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/d)
        productFromD of Product/d
          name of String/d
    ");
    insta::assert_snapshot!(qtps[4].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/d)
        productFromD of Product/d
          id of ID/d
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

    insta::assert_snapshot!(best_paths_per_leaf[0][0].pretty_print(&graph), @"root(Query) -(products)- Query/products -(topProducts)- ProductList/products -(products)- Product/products -(🔑🧩{id pid category{id tag}})- Product/price -(price)- Price/price -(price)- Float/price");
    insta::assert_snapshot!(best_paths_per_leaf[1][0].pretty_print(&graph), @"root(Query) -(products)- Query/products -(topProducts)- ProductList/products -(products)- Product/products -(id)- String/products");

    let qtps = paths_to_trees(&graph, &best_paths_per_leaf);

    // TODO: Understand why "tag" and "id" are not both under the same "category of Category/PRODUCTS"
    insta::assert_snapshot!(qtps[0].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/products)
        topProducts of ProductList/products
          products of Product/products
            🧩 [
              id of String/products
              🧩 [
                id of String/products
              ]
              🔑 Product/link
                pid of String/link
              category of Category/products
                tag of String/products
                id of String/products
            ]
            🔑 Product/price
              price of Price/price
                price of Float/price
    ");

    insta::assert_snapshot!(qtps[1].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/products)
        topProducts of ProductList/products
          products of Product/products
            id of String/products
    ");

    Ok(())
}
