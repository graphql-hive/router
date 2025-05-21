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
      @"root(Query) -(inventory)- Query/inventory -(products)- Product/inventory -(isExpensive🧩{price})- Boolean/inventory"
    );

    let qtps = paths_to_trees(&graph, &best_paths_per_leaf);
    let query_tree = QueryTree::merge_trees(qtps);

    insta::assert_snapshot!(query_tree.pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/inventory)
        products of Product/inventory
          🧩 [
            🧩 [
              upc of String/inventory
            ]
            🔑 Product/products
              price of Int/products
          ]
          isExpensive of Boolean/inventory
    ");

    let fetch_graph = build_fetch_graph_from_query_tree(&graph, query_tree)?;
    let query_plan = build_query_plan_from_fetch_graph(fetch_graph)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "inventory") {
          {products{__typename upc}}
        },
        Flatten(path: "products.@") {
          Fetch(service: "products") {
              __typename
              upc
            } =>
            {price}
          },
        },
        Flatten(path: "products.@") {
          Fetch(service: "inventory") {
              __typename
              price
              upc
            } =>
            {isExpensive}
          },
        },
      },
    },
    "#);

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
      @"root(Query) -(products)- Query/products -(products)- Product/products -(🔑🧩{upc})- Product/inventory -(isExpensive🧩{price})- Boolean/inventory"
    );

    let qtps = paths_to_trees(&graph, &best_paths_per_leaf);
    let query_tree = QueryTree::merge_trees(qtps);

    insta::assert_snapshot!(query_tree.pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/products)
        products of Product/products
          🧩 [
            upc of String/products
          ]
          🔑 Product/inventory
            🧩 [
              🧩 [
                upc of String/inventory
              ]
              🔑 Product/products
                price of Int/products
            ]
            isExpensive of Boolean/inventory
    ");

    let fetch_graph = build_fetch_graph_from_query_tree(&graph, query_tree)?;
    let query_plan = build_query_plan_from_fetch_graph(fetch_graph)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "products") {
          {products{__typename upc price}}
        },
        Flatten(path: "products.@") {
          Fetch(service: "inventory") {
              __typename
              price
              upc
            } =>
            {isExpensive}
          },
        },
      },
    },
    "#);

    Ok(())
}

#[test]
fn simplest_requires_with_local_sibling() -> Result<(), Box<dyn Error>> {
    init_logger();
    let graph = read_supergraph("fixture/tests/requires-local-sibling.supergraph.graphql");
    let document = parse_operation(
        r#"
        query {
          products {
            isExpensive
            isAvailable
          }
        }"#,
    );
    let operation = get_operation_to_execute(&document).expect("failed to locate operation");
    let best_paths_per_leaf = walk_operation(&graph, operation)?;
    assert_eq!(best_paths_per_leaf.len(), 2);
    assert_eq!(best_paths_per_leaf[0].len(), 1);
    assert_eq!(best_paths_per_leaf[1].len(), 1);

    let qtps = paths_to_trees(&graph, &best_paths_per_leaf);
    let query_tree = QueryTree::merge_trees(qtps);

    insta::assert_snapshot!(query_tree.pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/products)
        products of Product/products
          🧩 [
            upc of String/products
          ]
          🔑 Product/inventory
            isAvailable of Boolean/inventory
            🧩 [
              🧩 [
                upc of String/inventory
              ]
              🔑 Product/products
                price of Int/products
            ]
            isExpensive of Boolean/inventory
    ");

    let fetch_graph = build_fetch_graph_from_query_tree(&graph, query_tree)?;
    let query_plan = build_query_plan_from_fetch_graph(fetch_graph)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "products") {
          {products{__typename upc price}}
        },
        Flatten(path: "products.@") {
          Fetch(service: "inventory") {
              __typename
              price
              upc
            } =>
            {isExpensive isAvailable}
          },
        },
      },
    },
    "#);

    Ok(())
}

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
      @"root(Query) -(products)- Query/products -(products)- Product/products -(🔑🧩{upc})- Product/inventory -(shippingEstimate🧩{price weight})- Int/inventory"
    );

    let qtps = paths_to_trees(&graph, &best_paths_per_leaf);
    let query_tree = QueryTree::merge_trees(qtps);

    insta::assert_snapshot!(query_tree.pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/products)
        products of Product/products
          🧩 [
            upc of String/products
          ]
          🔑 Product/inventory
            🧩 [
              🧩 [
                upc of String/inventory
              ]
              🔑 Product/products
                price of Int/products
                weight of Int/products
            ]
            shippingEstimate of Int/inventory
    ");

    let fetch_graph = build_fetch_graph_from_query_tree(&graph, query_tree)?;
    let query_plan = build_query_plan_from_fetch_graph(fetch_graph)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "products") {
          {products{__typename upc price weight}}
        },
        Flatten(path: "products.@") {
          Fetch(service: "inventory") {
              __typename
              price
              weight
              upc
            } =>
            {shippingEstimate}
          },
        },
      },
    },
    "#);

    Ok(())
}

#[test]
fn two_fields_same_subgraph_same_requirement() -> Result<(), Box<dyn Error>> {
    init_logger();
    let graph = read_supergraph(
        "fixture/tests/two_fields_same_subgraph_same_requirement.supergraph.graphql",
    );
    let document = parse_operation(
        r#"
        query {
          products {
            shippingEstimate
            shippingEstimate2
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
      @"root(Query) -(products)- Query/products -(products)- Product/products -(🔑🧩{upc})- Product/inventory -(shippingEstimate2🧩{price weight})- String/inventory"
    );

    insta::assert_snapshot!(
      best_paths_per_leaf[1][0].pretty_print(&graph),
      @"root(Query) -(products)- Query/products -(products)- Product/products -(🔑🧩{upc})- Product/inventory -(shippingEstimate🧩{price weight})- Int/inventory"
    );

    let qtps = paths_to_trees(&graph, &best_paths_per_leaf);
    let query_tree = QueryTree::merge_trees(qtps);

    insta::assert_snapshot!(query_tree.pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/products)
        products of Product/products
          🧩 [
            upc of String/products
          ]
          🔑 Product/inventory
            🧩 [
              🧩 [
                upc of String/inventory
              ]
              🔑 Product/products
                price of Int/products
                weight of Int/products
            ]
            shippingEstimate2 of String/inventory
            🧩 [
              🧩 [
                upc of String/inventory
              ]
              🔑 Product/products
                price of Int/products
                weight of Int/products
            ]
            shippingEstimate of Int/inventory
    ");

    let fetch_graph = build_fetch_graph_from_query_tree(&graph, query_tree)?;
    let query_plan = build_query_plan_from_fetch_graph(fetch_graph)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "products") {
          {products{__typename upc price weight}}
        },
        Flatten(path: "products.@") {
          Fetch(service: "inventory") {
              __typename
              price
              weight
              upc
            } =>
            {shippingEstimate2 shippingEstimate}
          },
        },
      },
    },
    "#);

    Ok(())
}

#[test]
fn simple_requires_with_child() -> Result<(), Box<dyn Error>> {
    init_logger();
    let graph = read_supergraph("fixture/tests/simple_requires_with_child.supergraph.graphql");
    let document = parse_operation(
        r#"
        query {
          products {
            shippingEstimate {
              price
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
      @"root(Query) -(products)- Query/products -(products)- Product/products -(🔑🧩{upc})- Product/inventory -(shippingEstimate🧩{price weight})- Estimate/inventory -(price)- Int/inventory"
    );

    let qtps = paths_to_trees(&graph, &best_paths_per_leaf);
    let query_tree = QueryTree::merge_trees(qtps);

    insta::assert_snapshot!(query_tree.pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/products)
        products of Product/products
          🧩 [
            upc of String/products
          ]
          🔑 Product/inventory
            🧩 [
              🧩 [
                upc of String/inventory
              ]
              🔑 Product/products
                price of Int/products
                weight of Int/products
            ]
            shippingEstimate of Estimate/inventory
              price of Int/inventory
    ");

    let fetch_graph = build_fetch_graph_from_query_tree(&graph, query_tree)?;
    let query_plan = build_query_plan_from_fetch_graph(fetch_graph)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "products") {
          {products{__typename upc price weight}}
        },
        Flatten(path: "products.@") {
          Fetch(service: "inventory") {
              __typename
              price
              weight
              upc
            } =>
            {shippingEstimate{price}}
          },
        },
      },
    },
    "#);

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
    assert_eq!(best_paths_per_leaf[1].len(), 1);
    assert_eq!(best_paths_per_leaf[2].len(), 1);
    assert_eq!(best_paths_per_leaf[3].len(), 1);

    insta::assert_snapshot!(
      best_paths_per_leaf[0][0].pretty_print(&graph),
      @"root(Query) -(b)- Query/b -(b)- B/b -(a)- A/b -(nameInB🧩{name})- String/b"
    );
    insta::assert_snapshot!(
      best_paths_per_leaf[1][0].pretty_print(&graph),
      @"root(Query) -(b)- Query/b -(b)- B/b -(a)- A/b -(🔑🧩{id})- A/a -(name)- String/a"
    );
    insta::assert_snapshot!(
      best_paths_per_leaf[2][0].pretty_print(&graph),
      @"root(Query) -(b)- Query/b -(b)- B/b -(a)- A/b -(id)- ID/b"
    );
    insta::assert_snapshot!(
      best_paths_per_leaf[3][0].pretty_print(&graph),
      @"root(Query) -(b)- Query/b -(b)- B/b -(id)- ID/b"
    );

    let qtps = paths_to_trees(&graph, &best_paths_per_leaf);

    insta::assert_snapshot!(qtps[0].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/b)
        b of B/b
          a of A/b
            🧩 [
              🧩 [
                id of ID/b
              ]
              🔑 A/a
                name of String/a
            ]
            nameInB of String/b
    ");

    insta::assert_snapshot!(qtps[1].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/b)
        b of B/b
          a of A/b
            🧩 [
              id of ID/b
            ]
            🔑 A/a
              name of String/a
    ");

    insta::assert_snapshot!(qtps[2].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/b)
        b of B/b
          a of A/b
            id of ID/b
    ");

    insta::assert_snapshot!(qtps[3].pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/b)
        b of B/b
          id of ID/b
    ");

    let query_tree = QueryTree::merge_trees(qtps);

    insta::assert_snapshot!(query_tree.pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/b)
        b of B/b
          a of A/b
            🧩 [
              🧩 [
                id of ID/b
              ]
              🔑 A/a
                name of String/a
            ]
            nameInB of String/b
            🧩 [
              id of ID/b
            ]
            🔑 A/a
              name of String/a
            id of ID/b
          id of ID/b
    ");

    let fetch_graph = build_fetch_graph_from_query_tree(&graph, query_tree)?;
    let query_plan = build_query_plan_from_fetch_graph(fetch_graph)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "store") {
          {products{__typename id}}
        },
        Flatten(path: "products") {
          Fetch(service: "info") {
              __typename
              id
            } =>
            {isAvailable uuid}
          },
        },
        Flatten(path: "products") {
          Fetch(service: "cost") {
              __typename
              uuid
            } =>
            {price{currency amount}}
          },
        },
      },
    },
    "#);

    Ok(())
}
