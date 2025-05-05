use crate::{
    parse_operation,
    planner::walker::walk_operation,
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
    let best_paths_per_leaf = walk_operation(&graph, operation)?;
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
      @"root(Query) -(PRICE)- Query/PRICE -(product)- Product/PRICE -(price)- Price/PRICE -(currency)- String/PRICE"
    );
    insta::assert_snapshot!(
      best_paths_per_leaf[1][0].pretty_print(&graph),
      @"root(Query) -(PRICE)- Query/PRICE -(product)- Product/PRICE -(price)- Price/PRICE -(amount)- Int/PRICE"
    );
    insta::assert_snapshot!(
      best_paths_per_leaf[2][0].pretty_print(&graph),
      @"root(Query) -(PRICE)- Query/PRICE -(product)- Product/PRICE -(price)- Price/PRICE -(id)- ID/PRICE"
    );
    insta::assert_snapshot!(
      best_paths_per_leaf[3][0].pretty_print(&graph),
      @"root(Query) -(CATEGORY)- Query/CATEGORY -(product)- Product/CATEGORY -(category)- Category/CATEGORY -(name)- String/CATEGORY"
    );
    insta::assert_snapshot!(
      best_paths_per_leaf[4][0].pretty_print(&graph),
      @"root(Query) -(CATEGORY)- Query/CATEGORY -(product)- Product/CATEGORY -(category)- Category/CATEGORY -(id)- ID/CATEGORY"
    );
    insta::assert_snapshot!(
      best_paths_per_leaf[5][0].pretty_print(&graph),
      @"root(Query) -(NAME)- Query/NAME -(product)- Product/NAME -(name)- Name/NAME -(model)- String/NAME"
    );
    insta::assert_snapshot!(
      best_paths_per_leaf[6][0].pretty_print(&graph),
      @"root(Query) -(NAME)- Query/NAME -(product)- Product/NAME -(name)- Name/NAME -(brand)- String/NAME"
    );
    insta::assert_snapshot!(
      best_paths_per_leaf[7][0].pretty_print(&graph),
      @"root(Query) -(NAME)- Query/NAME -(product)- Product/NAME -(name)- Name/NAME -(id)- ID/NAME"
    );

    let mut as_strs = best_paths_per_leaf[8]
        .iter()
        .map(|p| p.pretty_print(&graph))
        .collect::<Vec<String>>();
    as_strs.sort();

    insta::assert_snapshot!(
      as_strs[0],
      @"root(Query) -(CATEGORY)- Query/CATEGORY -(product)- Product/CATEGORY -(id)- ID/CATEGORY"
    );
    insta::assert_snapshot!(
      as_strs[1],
      @"root(Query) -(NAME)- Query/NAME -(product)- Product/NAME -(id)- ID/NAME"
    );
    insta::assert_snapshot!(
      as_strs[2],
      @"root(Query) -(PRICE)- Query/PRICE -(product)- Product/PRICE -(id)- ID/PRICE"
    );

    let qtps = paths_to_trees(&graph, &best_paths_per_leaf);

    insta::assert_snapshot!(qtps[0].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/PRICE)
        product of Product/PRICE
          price of Price/PRICE
            currency of String/PRICE
    ");
    insta::assert_snapshot!(qtps[1].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/PRICE)
        product of Product/PRICE
          price of Price/PRICE
            amount of Int/PRICE
    ");
    insta::assert_snapshot!(qtps[2].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/PRICE)
        product of Product/PRICE
          price of Price/PRICE
            id of ID/PRICE
    ");
    insta::assert_snapshot!(qtps[3].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/CATEGORY)
        product of Product/CATEGORY
          category of Category/CATEGORY
            name of String/CATEGORY
    ");
    insta::assert_snapshot!(qtps[4].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/CATEGORY)
        product of Product/CATEGORY
          category of Category/CATEGORY
            id of ID/CATEGORY
    ");
    insta::assert_snapshot!(qtps[5].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/NAME)
        product of Product/NAME
          name of Name/NAME
            model of String/NAME
    ");
    insta::assert_snapshot!(qtps[6].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/NAME)
        product of Product/NAME
          name of Name/NAME
            brand of String/NAME
    ");
    insta::assert_snapshot!(qtps[7].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/NAME)
        product of Product/NAME
          name of Name/NAME
            id of ID/NAME
    ");
    insta::assert_snapshot!(qtps[8].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/PRICE)
        product of Product/PRICE
          id of ID/PRICE
    ");

    Ok(())
}
