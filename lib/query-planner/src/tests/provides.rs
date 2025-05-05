use crate::{
    parse_operation,
    planner::walker::walk_operation,
    tests::testkit::{init_logger, read_supergraph},
    utils::operation_utils::get_operation_to_execute,
};
use std::error::Error;

#[test]
fn simple_provides() -> Result<(), Box<dyn Error>> {
    init_logger();
    let graph = read_supergraph("fixture/tests/simple-provides.supergraph.graphql");
    let document = parse_operation(
        r#"
        query {
          products {
            reviews {
              author {
                username
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
      @"root(Query) -(PRODUCTS)- Query/PRODUCTS -(products)- Product/PRODUCTS -(🔑🧩upc)- Product/REVIEWS -(reviews)- Review/REVIEWS -(author)- (User/REVIEWS).view1 -(username)- String/REVIEWS"
    );

    Ok(())
}

#[test]
fn nested_provides() -> Result<(), Box<dyn Error>> {
    init_logger();
    let graph = read_supergraph("fixture/tests/nested-provides.supergraph.graphql");
    let document = parse_operation(
        r#"
        query {
          products {
            id
            categories {
              id
              name
            }
          }
        }"#,
    );

    let operation = get_operation_to_execute(&document).expect("failed to locate operation");
    let best_paths_per_leaf = walk_operation(&graph, operation)?;
    assert_eq!(best_paths_per_leaf.len(), 3);
    assert_eq!(best_paths_per_leaf[0].len(), 1);
    assert_eq!(best_paths_per_leaf[1].len(), 1);
    assert_eq!(best_paths_per_leaf[2].len(), 1);

    Ok(())
}
