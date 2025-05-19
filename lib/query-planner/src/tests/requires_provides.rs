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
fn simple_requires_provides() -> Result<(), Box<dyn Error>> {
    init_logger();
    let graph = read_supergraph("fixture/tests/requires-provides.supergraph.graphql");
    let document = parse_operation(
        r#"
        query {
          me {
            reviews {
              id
              author {
                id
                username
              }
              product {
                inStock
              }
            }
          }
        }"#,
    );
    let operation = get_operation_to_execute(&document).expect("failed to locate operation");
    let best_paths_per_leaf = walk_operation(&graph, operation)?;
    assert_eq!(best_paths_per_leaf.len(), 4);

    insta::assert_snapshot!(best_paths_per_leaf[0][0].pretty_print(&graph), @"root(Query) -(accounts)- Query/accounts -(me)- User/accounts -(🔑🧩{id})- User/reviews -(reviews)- Review/reviews -(product)- Product/reviews -(🔑🧩{upc})- Product/inventory -(inStock)- Boolean/inventory");
    insta::assert_snapshot!(best_paths_per_leaf[1][0].pretty_print(&graph), @"root(Query) -(accounts)- Query/accounts -(me)- User/accounts -(🔑🧩{id})- User/reviews -(reviews)- Review/reviews -(author)- User/reviews/1 -(username)- String/reviews/1");
    insta::assert_snapshot!(best_paths_per_leaf[2][0].pretty_print(&graph), @"root(Query) -(accounts)- Query/accounts -(me)- User/accounts -(🔑🧩{id})- User/reviews -(reviews)- Review/reviews -(author)- User/reviews -(id)- ID/reviews");
    insta::assert_snapshot!(best_paths_per_leaf[3][0].pretty_print(&graph), @"root(Query) -(accounts)- Query/accounts -(me)- User/accounts -(🔑🧩{id})- User/reviews -(reviews)- Review/reviews -(id)- ID/reviews");

    let qtps = paths_to_trees(&graph, &best_paths_per_leaf);

    let query_tree = QueryTree::merge_trees(qtps);

    insta::assert_snapshot!(query_tree.pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/accounts)
        me of User/accounts
          🧩 [
            id of ID/accounts
          ]
          🔑 User/reviews
            reviews of Review/reviews
              product of Product/reviews
                🧩 [
                  upc of String/reviews
                ]
                🔑 Product/inventory
                  inStock of Boolean/inventory
              author of User/reviews/1
                username of String/reviews/1
              author of User/reviews
                id of ID/reviews
              id of ID/reviews
    ");

    let fetch_graph = build_fetch_graph_from_query_tree(&graph, query_tree)?;

    // TODO: [2] is missing "id" in input
    // TODO: [3] is missing "upc" in input
    insta::assert_snapshot!(format!("{}", fetch_graph), @r"
    Nodes:
    [1] Query/accounts {} → {me{id}} at $.
    [2] User/reviews {__typename id} → {reviews{product{upc} author{username id} id}} at $.me
    [3] Product/inventory {__typename upc} → {inStock} at $.me.reviews.@.product

    Tree:
    [1]
      [2]
        [3]
    ");

    Ok(())
}
