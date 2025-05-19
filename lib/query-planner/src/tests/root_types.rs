use crate::{
    parse_operation,
    planner::{
        fetch::fetch_graph::build_fetch_graph_from_query_tree,
        query_plan::build_query_plan_from_fetch_graph, tree::query_tree::QueryTree,
        walker::walk_operation,
    },
    tests::testkit::{init_logger, paths_to_trees, read_supergraph},
    utils::operation_utils::get_operation_to_execute,
};
use std::error::Error;

#[test]
fn shared_root() -> Result<(), Box<dyn Error>> {
    init_logger();
    let graph = read_supergraph("fixture/tests/shared-root.supergraph.graphql");
    let document = parse_operation(
        r#"
        query {
          product {
            id
            name {
              id
              brand
              model
            }
            category {
              id
              name
            }
            price {
              id
              amount
              currency
            }
          }
        }"#,
    );
    let operation = get_operation_to_execute(&document).expect("failed to locate operation");
    let mut best_paths_per_leaf = walk_operation(&graph, operation)?;
    assert_eq!(best_paths_per_leaf.len(), 9);
    assert_eq!(best_paths_per_leaf[0].len(), 1);
    assert_eq!(best_paths_per_leaf[1].len(), 1);
    assert_eq!(best_paths_per_leaf[2].len(), 1);
    assert_eq!(best_paths_per_leaf[3].len(), 1);
    assert_eq!(best_paths_per_leaf[4].len(), 1);
    assert_eq!(best_paths_per_leaf[5].len(), 1);
    assert_eq!(best_paths_per_leaf[6].len(), 1);
    assert_eq!(best_paths_per_leaf[7].len(), 1);
    assert_eq!(best_paths_per_leaf[8].len(), 3);

    insta::assert_snapshot!(
      best_paths_per_leaf[0][0].pretty_print(&graph),
      @"root(Query) -(price)- Query/price -(product)- Product/price -(price)- Price/price -(currency)- String/price"
    );
    insta::assert_snapshot!(
      best_paths_per_leaf[1][0].pretty_print(&graph),
      @"root(Query) -(price)- Query/price -(product)- Product/price -(price)- Price/price -(amount)- Int/price"
    );
    insta::assert_snapshot!(
      best_paths_per_leaf[2][0].pretty_print(&graph),
      @"root(Query) -(price)- Query/price -(product)- Product/price -(price)- Price/price -(id)- ID/price"
    );
    insta::assert_snapshot!(
      best_paths_per_leaf[3][0].pretty_print(&graph),
      @"root(Query) -(category)- Query/category -(product)- Product/category -(category)- Category/category -(name)- String/category"
    );
    insta::assert_snapshot!(
      best_paths_per_leaf[4][0].pretty_print(&graph),
      @"root(Query) -(category)- Query/category -(product)- Product/category -(category)- Category/category -(id)- ID/category"
    );
    insta::assert_snapshot!(
      best_paths_per_leaf[5][0].pretty_print(&graph),
      @"root(Query) -(name)- Query/name -(product)- Product/name -(name)- Name/name -(model)- String/name"
    );
    insta::assert_snapshot!(
      best_paths_per_leaf[6][0].pretty_print(&graph),
      @"root(Query) -(name)- Query/name -(product)- Product/name -(name)- Name/name -(brand)- String/name"
    );
    insta::assert_snapshot!(
      best_paths_per_leaf[7][0].pretty_print(&graph),
      @"root(Query) -(name)- Query/name -(product)- Product/name -(name)- Name/name -(id)- ID/name"
    );

    best_paths_per_leaf[8].sort_by_key(|a| a.pretty_print(&graph));

    insta::assert_snapshot!(
      best_paths_per_leaf[8][0].pretty_print(&graph),
      @"root(Query) -(category)- Query/category -(product)- Product/category -(id)- ID/category"
    );
    insta::assert_snapshot!(
      best_paths_per_leaf[8][1].pretty_print(&graph),
      @"root(Query) -(name)- Query/name -(product)- Product/name -(id)- ID/name"
    );
    insta::assert_snapshot!(
      best_paths_per_leaf[8][2].pretty_print(&graph),
      @"root(Query) -(price)- Query/price -(product)- Product/price -(id)- ID/price"
    );

    let qtps = paths_to_trees(&graph, &best_paths_per_leaf);

    insta::assert_snapshot!(qtps[0].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/price)
        product of Product/price
          price of Price/price
            currency of String/price
    ");
    insta::assert_snapshot!(qtps[1].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/price)
        product of Product/price
          price of Price/price
            amount of Int/price
    ");
    insta::assert_snapshot!(qtps[2].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/price)
        product of Product/price
          price of Price/price
            id of ID/price
    ");
    insta::assert_snapshot!(qtps[3].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/category)
        product of Product/category
          category of Category/category
            name of String/category
    ");
    insta::assert_snapshot!(qtps[4].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/category)
        product of Product/category
          category of Category/category
            id of ID/category
    ");
    insta::assert_snapshot!(qtps[5].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/name)
        product of Product/name
          name of Name/name
            model of String/name
    ");
    insta::assert_snapshot!(qtps[6].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/name)
        product of Product/name
          name of Name/name
            brand of String/name
    ");
    insta::assert_snapshot!(qtps[7].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/name)
        product of Product/name
          name of Name/name
            id of ID/name
    ");
    insta::assert_snapshot!(qtps[8].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/category)
        product of Product/category
          id of ID/category
    ");

    let query_tree = QueryTree::merge_trees(qtps);
    let fetch_graph = build_fetch_graph_from_query_tree(&graph, query_tree)?;

    insta::assert_snapshot!(format!("{}", fetch_graph), @r"
    Nodes:
    [1] Query/price {} → {product} at $.
    [2] Query/category {} → {product} at $.
    [3] Query/name {} → {product} at $.

    Tree:
    [1]
    [2]
    [3]
    ");

    let query_plan = build_query_plan_from_fetch_graph(fetch_graph)?;
    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Parallel {
        Fetch(service: "name") {
          } =>
          {
          }
        },
        Fetch(service: "category") {
          } =>
          {
          }
        },
        Fetch(service: "price") {
          } =>
          {
          }
        },
      },
    },
    "#);

    Ok(())
}
