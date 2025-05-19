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
      @"root(Query) -(products)- Query/products -(products)- Product/products -(🔑🧩{upc})- Product/reviews -(reviews)- Review/reviews -(author)- User/reviews/1 -(username)- String/reviews/1"
    );

    let qtps = paths_to_trees(&graph, &best_paths_per_leaf);

    insta::assert_snapshot!(qtps[0].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/products)
        products of Product/products
          🧩 [
            upc of String/products
          ]
          🔑 Product/reviews
            reviews of Review/reviews
              author of User/reviews/1
                username of String/reviews/1
    ");

    let query_tree = QueryTree::merge_trees(qtps);
    let fetch_graph = build_fetch_graph_from_query_tree(&graph, query_tree)?;

    insta::assert_snapshot!(format!("{}", fetch_graph), @r"
    Nodes:
    [1] Query/products {} → {products} at $.
    [2] Product/reviews {__typename} → {reviews} at $.products.@

    Tree:
    [1]
      [2]
    ");

    let query_plan = build_query_plan_from_fetch_graph(fetch_graph)?;
    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "products") {
          } =>
          {
          }
        },
        Flatten(path: "products.@") {
          Fetch(service: "reviews") {
            } =>
            {
            }
          },
        },
      },
    },
    "#);

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

    let qtps = paths_to_trees(&graph, &best_paths_per_leaf);

    insta::assert_snapshot!(qtps[0].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/category)
        products of Product/category/1
          categories of Category/category/1
            name of String/category/1
    ");
    insta::assert_snapshot!(qtps[1].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/category)
        products of Product/category/1
          categories of Category/category/1
            id of ID/category/1
    ");
    insta::assert_snapshot!(qtps[2].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/category)
        products of Product/category
          id of ID/category
    ");

    let query_tree = QueryTree::merge_trees(qtps);
    let fetch_graph = build_fetch_graph_from_query_tree(&graph, query_tree)?;

    insta::assert_snapshot!(format!("{}", fetch_graph), @r"
    Nodes:
    [1] Query/category {} → {products} at $.

    Tree:
    [1]
    ");

    let query_plan = build_query_plan_from_fetch_graph(fetch_graph)?;
    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "category") {
        } =>
        {
        }
      },
    },
    "#);

    Ok(())
}
