use crate::{
    parse_operation,
    planner::{tree::query_tree::QueryTree, walker::walk_operation},
    tests::testkit::{init_logger, read_supergraph},
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
                }
                isAvailable
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
      @"root(Query) -(STORE)- Query/STORE -(products)- Product/STORE -(🔑🧩id)- Product/INFO -(isAvailable)- Boolean/INFO"
    );
    insta::assert_snapshot!(
      best_paths_per_leaf[1][0].pretty_print(&graph),
      @"root(Query) -(STORE)- Query/STORE -(products)- Product/STORE -(🔑🧩uuid)- Product/COST -(price)- Price/COST -(amount)- Float/COST"
    );

    let qtps = best_paths_per_leaf
        .iter()
        .map(|paths| {
            QueryTree::from_path(&graph, &paths[0])
                .expect("expected tree to be built but it failed")
        })
        .collect::<Vec<_>>();

    insta::assert_snapshot!(qtps[0].pretty_print(&graph)?, @r"
    root(Query)
      🚪 STORE
        products of Product/STORE
          🧩 [
            id of ID/STORE
          ]
          🔑 Product/INFO
            isAvailable of Boolean/INFO
    ");

    insta::assert_snapshot!(qtps[1].pretty_print(&graph)?, @r"
    root(Query)
      🚪 STORE
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
      @"root(Query) -(A)- Query/A -(products)- Product/A -(🔑🧩id pid)- Product/C -(category)- Category/C -(details)- CategoryDetails/C -(products)- Int/C"
    );

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

    insta::assert_snapshot!(best_paths_per_leaf[0][0].pretty_print(&graph), @"root(Query) -(D)- Query/D -(productFromD)- Product/D -(🔑🧩id)- Product/A -(category)- Category/A -(details)- String/A");
    insta::assert_snapshot!(best_paths_per_leaf[1][0].pretty_print(&graph), @"root(Query) -(D)- Query/D -(productFromD)- Product/D -(🔑🧩id)- Product/B -(category)- Category/B -(🔑🧩id)- Category/C -(name)- String/C");
    insta::assert_snapshot!(best_paths_per_leaf[2][0].pretty_print(&graph), @"root(Query) -(D)- Query/D -(productFromD)- Product/D -(🔑🧩id)- Product/B -(category)- Category/B -(id)- ID/B");
    insta::assert_snapshot!(best_paths_per_leaf[3][0].pretty_print(&graph), @"root(Query) -(D)- Query/D -(productFromD)- Product/D -(name)- String/D");
    insta::assert_snapshot!(best_paths_per_leaf[4][0].pretty_print(&graph), @"root(Query) -(D)- Query/D -(productFromD)- Product/D -(id)- ID/D");

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

    insta::assert_snapshot!(best_paths_per_leaf[0][0].pretty_print(&graph), @"root(Query) -(PRODUCTS)- Query/PRODUCTS -(topProducts)- ProductList/PRODUCTS -(products)- Product/PRODUCTS -(🔑🧩id pid category{id tag})- Product/PRICE -(price)- Price/PRICE -(price)- Float/PRICE");
    insta::assert_snapshot!(best_paths_per_leaf[1][0].pretty_print(&graph), @"root(Query) -(PRODUCTS)- Query/PRODUCTS -(topProducts)- ProductList/PRODUCTS -(products)- Product/PRODUCTS -(id)- String/PRODUCTS");

    Ok(())
}
