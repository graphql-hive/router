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
fn one() -> Result<(), Box<dyn Error>> {
    init_logger();
    let graph = read_supergraph("fixture/tests/requires_requires.supergraph.graphql");
    let document = parse_operation(
        r#"
        query {
          product {
            canAffordWithDiscount
          }
        }"#,
    );
    let operation = get_operation_to_execute(&document).expect("failed to locate operation");
    let best_paths_per_leaf = walk_operation(&graph, operation)?;
    assert_eq!(best_paths_per_leaf.len(), 1);

    insta::assert_snapshot!(best_paths_per_leaf[0][0].pretty_print(&graph), @"root(Query) -(b)- Query/b -(product)- Product/b -(🔑🧩{id})- Product/d -(canAffordWithDiscount🧩{isExpensiveWithDiscount})- Boolean/d");

    let qtps = paths_to_trees(&graph, &best_paths_per_leaf);
    let query_tree = QueryTree::merge_trees(qtps);

    insta::assert_snapshot!(query_tree.pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/b)
        product of Product/b
          🧩 [
            id of ID/b
          ]
          🔑 Product/d
            🧩 [
              🧩 [
                id of ID/d
              ]
              🔑 Product/c
                🧩 [
                  🧩 [
                    id of ID/c
                  ]
                  🔑 Product/b
                    hasDiscount of Boolean/b
                ]
                isExpensiveWithDiscount of Boolean/c
            ]
            canAffordWithDiscount of Boolean/d
    ");

    let fetch_graph = build_fetch_graph_from_query_tree(&graph, query_tree)?;
    let query_plan = build_query_plan_from_fetch_graph(fetch_graph)?;
    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "b") {
          {product{__typename id hasDiscount}}
        },
        Flatten(path: "product") {
          Fetch(service: "c") {
              __typename
              hasDiscount
              id
            } =>
            {isExpensiveWithDiscount}
          },
        },
        Flatten(path: "product") {
          Fetch(service: "d") {
              __typename
              isExpensiveWithDiscount
              id
            } =>
            {canAffordWithDiscount}
          },
        },
      },
    },
    "#);

    Ok(())
}

#[test]
fn one_with_one_local() -> Result<(), Box<dyn Error>> {
    init_logger();
    let graph = read_supergraph("fixture/tests/requires_requires.supergraph.graphql");
    let document = parse_operation(
        r#"
        query {
          product {
            fieldInD
            canAffordWithDiscount
          }
        }"#,
    );
    let operation = get_operation_to_execute(&document).expect("failed to locate operation");
    let best_paths_per_leaf = walk_operation(&graph, operation)?;
    assert_eq!(best_paths_per_leaf.len(), 2);
    assert_eq!(best_paths_per_leaf[0].len(), 1);
    assert_eq!(best_paths_per_leaf[1].len(), 1);

    insta::assert_snapshot!(best_paths_per_leaf[0][0].pretty_print(&graph), @"root(Query) -(b)- Query/b -(product)- Product/b -(🔑🧩{id})- Product/d -(canAffordWithDiscount🧩{isExpensiveWithDiscount})- Boolean/d");
    insta::assert_snapshot!(best_paths_per_leaf[1][0].pretty_print(&graph), @"root(Query) -(b)- Query/b -(product)- Product/b -(🔑🧩{id})- Product/d -(fieldInD)- String/d");

    let qtps = paths_to_trees(&graph, &best_paths_per_leaf);
    let query_tree = QueryTree::merge_trees(qtps);

    insta::assert_snapshot!(query_tree.pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/b)
        product of Product/b
          🧩 [
            id of ID/b
          ]
          🔑 Product/d
            🧩 [
              🧩 [
                id of ID/d
              ]
              🔑 Product/c
                🧩 [
                  🧩 [
                    id of ID/c
                  ]
                  🔑 Product/b
                    hasDiscount of Boolean/b
                ]
                isExpensiveWithDiscount of Boolean/c
            ]
            canAffordWithDiscount of Boolean/d
            fieldInD of String/d
    ");

    let fetch_graph = build_fetch_graph_from_query_tree(&graph, query_tree)?;
    let query_plan = build_query_plan_from_fetch_graph(fetch_graph)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "b") {
          {product{__typename id hasDiscount}}
        },
        Parallel {
          Flatten(path: "product") {
            Fetch(service: "c") {
                __typename
                hasDiscount
                id
              } =>
              {isExpensiveWithDiscount}
            },
          },
          Flatten(path: "product") {
            Fetch(service: "d") {
                __typename
                id
              } =>
              {fieldInD}
            },
          },
        },
        Flatten(path: "product") {
          Fetch(service: "d") {
              __typename
              isExpensiveWithDiscount
              id
            } =>
            {canAffordWithDiscount}
          },
        },
      },
    },
    "#);

    Ok(())
}

#[test]
fn two_fields_with_the_same_requirements() -> Result<(), Box<dyn Error>> {
    init_logger();
    let graph = read_supergraph("fixture/tests/requires_requires.supergraph.graphql");
    let document = parse_operation(
        r#"
        query {
          product {
            canAffordWithDiscount
            canAffordWithDiscount2
          }
        }"#,
    );
    let operation = get_operation_to_execute(&document).expect("failed to locate operation");
    let best_paths_per_leaf = walk_operation(&graph, operation)?;
    assert_eq!(best_paths_per_leaf.len(), 2);
    assert_eq!(best_paths_per_leaf[0].len(), 1);
    assert_eq!(best_paths_per_leaf[1].len(), 1);

    insta::assert_snapshot!(best_paths_per_leaf[0][0].pretty_print(&graph), @"root(Query) -(b)- Query/b -(product)- Product/b -(🔑🧩{id})- Product/d -(canAffordWithDiscount2🧩{isExpensiveWithDiscount})- Boolean/d");
    insta::assert_snapshot!(best_paths_per_leaf[1][0].pretty_print(&graph), @"root(Query) -(b)- Query/b -(product)- Product/b -(🔑🧩{id})- Product/d -(canAffordWithDiscount🧩{isExpensiveWithDiscount})- Boolean/d");

    let qtps = paths_to_trees(&graph, &best_paths_per_leaf);
    let query_tree = QueryTree::merge_trees(qtps);

    insta::assert_snapshot!(query_tree.pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/b)
        product of Product/b
          🧩 [
            id of ID/b
          ]
          🔑 Product/d
            🧩 [
              🧩 [
                id of ID/d
              ]
              🔑 Product/c
                🧩 [
                  🧩 [
                    id of ID/c
                  ]
                  🔑 Product/b
                    hasDiscount of Boolean/b
                ]
                isExpensiveWithDiscount of Boolean/c
            ]
            canAffordWithDiscount2 of Boolean/d
            🧩 [
              🧩 [
                id of ID/d
              ]
              🔑 Product/c
                🧩 [
                  🧩 [
                    id of ID/c
                  ]
                  🔑 Product/b
                    hasDiscount of Boolean/b
                ]
                isExpensiveWithDiscount of Boolean/c
            ]
            canAffordWithDiscount of Boolean/d
    ");

    let fetch_graph = build_fetch_graph_from_query_tree(&graph, query_tree)?;
    let query_plan = build_query_plan_from_fetch_graph(fetch_graph)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "b") {
          {product{__typename id hasDiscount}}
        },
        Flatten(path: "product") {
          Fetch(service: "c") {
              __typename
              hasDiscount
              id
            } =>
            {isExpensiveWithDiscount}
          },
        },
        Flatten(path: "product") {
          Fetch(service: "d") {
              __typename
              isExpensiveWithDiscount
              id
            } =>
            {canAffordWithDiscount canAffordWithDiscount2}
          },
        },
      },
    },
    "#);

    Ok(())
}

#[test]
fn one_more() -> Result<(), Box<dyn Error>> {
    init_logger();
    let graph = read_supergraph("fixture/tests/requires_requires.supergraph.graphql");
    let document = parse_operation(
        r#"
        query {
          product {
            canAfford
          }
        }"#,
    );
    let operation = get_operation_to_execute(&document).expect("failed to locate operation");
    let best_paths_per_leaf = walk_operation(&graph, operation)?;
    assert_eq!(best_paths_per_leaf.len(), 1);
    assert_eq!(best_paths_per_leaf[0].len(), 1);

    insta::assert_snapshot!(best_paths_per_leaf[0][0].pretty_print(&graph), @"root(Query) -(b)- Query/b -(product)- Product/b -(🔑🧩{id})- Product/d -(canAfford🧩{isExpensive})- Boolean/d");

    let qtps = paths_to_trees(&graph, &best_paths_per_leaf);
    let query_tree = QueryTree::merge_trees(qtps);

    insta::assert_snapshot!(query_tree.pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/b)
        product of Product/b
          🧩 [
            id of ID/b
          ]
          🔑 Product/d
            🧩 [
              🧩 [
                id of ID/d
              ]
              🔑 Product/c
                🧩 [
                  🧩 [
                    id of ID/c
                  ]
                  🔑 Product/a
                    price of Float/a
                ]
                isExpensive of Boolean/c
            ]
            canAfford of Boolean/d
    ");

    let fetch_graph = build_fetch_graph_from_query_tree(&graph, query_tree)?;
    let query_plan = build_query_plan_from_fetch_graph(fetch_graph)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "b") {
          {product{__typename id}}
        },
        Flatten(path: "product") {
          Fetch(service: "a") {
              __typename
              id
            } =>
            {price}
          },
        },
        Flatten(path: "product") {
          Fetch(service: "c") {
              __typename
              price
              id
            } =>
            {isExpensive}
          },
        },
        Flatten(path: "product") {
          Fetch(service: "d") {
              __typename
              isExpensive
              id
            } =>
            {canAfford}
          },
        },
      },
    },
    "#);

    Ok(())
}

#[test]
fn another_two_fields_with_the_same_requirements() -> Result<(), Box<dyn Error>> {
    init_logger();
    let graph = read_supergraph("fixture/tests/requires_requires.supergraph.graphql");
    let document = parse_operation(
        r#"
        query {
          product {
            canAfford
            canAfford2
          }
        }"#,
    );
    let operation = get_operation_to_execute(&document).expect("failed to locate operation");
    let best_paths_per_leaf = walk_operation(&graph, operation)?;
    assert_eq!(best_paths_per_leaf.len(), 2);
    assert_eq!(best_paths_per_leaf[0].len(), 1);
    assert_eq!(best_paths_per_leaf[1].len(), 1);

    insta::assert_snapshot!(best_paths_per_leaf[0][0].pretty_print(&graph), @"root(Query) -(b)- Query/b -(product)- Product/b -(🔑🧩{id})- Product/d -(canAfford2🧩{isExpensive})- Boolean/d");
    insta::assert_snapshot!(best_paths_per_leaf[1][0].pretty_print(&graph), @"root(Query) -(b)- Query/b -(product)- Product/b -(🔑🧩{id})- Product/d -(canAfford🧩{isExpensive})- Boolean/d");

    let qtps = paths_to_trees(&graph, &best_paths_per_leaf);
    let query_tree = QueryTree::merge_trees(qtps);

    insta::assert_snapshot!(query_tree.pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/b)
        product of Product/b
          🧩 [
            id of ID/b
          ]
          🔑 Product/d
            🧩 [
              🧩 [
                id of ID/d
              ]
              🔑 Product/c
                🧩 [
                  🧩 [
                    id of ID/c
                  ]
                  🔑 Product/a
                    price of Float/a
                ]
                isExpensive of Boolean/c
            ]
            canAfford2 of Boolean/d
            🧩 [
              🧩 [
                id of ID/d
              ]
              🔑 Product/c
                🧩 [
                  🧩 [
                    id of ID/c
                  ]
                  🔑 Product/a
                    price of Float/a
                ]
                isExpensive of Boolean/c
            ]
            canAfford of Boolean/d
    ");

    let fetch_graph = build_fetch_graph_from_query_tree(&graph, query_tree)?;
    let query_plan = build_query_plan_from_fetch_graph(fetch_graph)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "b") {
          {product{__typename id}}
        },
        Flatten(path: "product") {
          Fetch(service: "a") {
              __typename
              id
            } =>
            {price}
          },
        },
        Flatten(path: "product") {
          Fetch(service: "c") {
              __typename
              price
              id
            } =>
            {isExpensive}
          },
        },
        Flatten(path: "product") {
          Fetch(service: "d") {
              __typename
              isExpensive
              id
            } =>
            {canAfford canAfford2}
          },
        },
      },
    },
    "#);

    Ok(())
}

#[test]
fn two_fields() -> Result<(), Box<dyn Error>> {
    init_logger();
    let graph = read_supergraph("fixture/tests/requires_requires.supergraph.graphql");
    let document = parse_operation(
        r#"
        query {
          product {
            canAffordWithDiscount
            canAfford
          }
        }"#,
    );
    let operation = get_operation_to_execute(&document).expect("failed to locate operation");
    let best_paths_per_leaf = walk_operation(&graph, operation)?;
    assert_eq!(best_paths_per_leaf.len(), 2);
    assert_eq!(best_paths_per_leaf[0].len(), 1);
    assert_eq!(best_paths_per_leaf[1].len(), 1);

    insta::assert_snapshot!(best_paths_per_leaf[0][0].pretty_print(&graph), @"root(Query) -(b)- Query/b -(product)- Product/b -(🔑🧩{id})- Product/d -(canAfford🧩{isExpensive})- Boolean/d");
    insta::assert_snapshot!(best_paths_per_leaf[1][0].pretty_print(&graph), @"root(Query) -(b)- Query/b -(product)- Product/b -(🔑🧩{id})- Product/d -(canAffordWithDiscount🧩{isExpensiveWithDiscount})- Boolean/d");

    let qtps = paths_to_trees(&graph, &best_paths_per_leaf);
    let query_tree = QueryTree::merge_trees(qtps);

    insta::assert_snapshot!(query_tree.pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/b)
        product of Product/b
          🧩 [
            id of ID/b
          ]
          🔑 Product/d
            🧩 [
              🧩 [
                id of ID/d
              ]
              🔑 Product/c
                🧩 [
                  🧩 [
                    id of ID/c
                  ]
                  🔑 Product/a
                    price of Float/a
                ]
                isExpensive of Boolean/c
            ]
            canAfford of Boolean/d
            🧩 [
              🧩 [
                id of ID/d
              ]
              🔑 Product/c
                🧩 [
                  🧩 [
                    id of ID/c
                  ]
                  🔑 Product/b
                    hasDiscount of Boolean/b
                ]
                isExpensiveWithDiscount of Boolean/c
            ]
            canAffordWithDiscount of Boolean/d
    ");

    let fetch_graph = build_fetch_graph_from_query_tree(&graph, query_tree)?;
    let query_plan = build_query_plan_from_fetch_graph(fetch_graph)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "b") {
          {product{__typename id hasDiscount}}
        },
        Parallel {
          Flatten(path: "product") {
            Fetch(service: "c") {
                __typename
                hasDiscount
                id
              } =>
              {isExpensiveWithDiscount}
            },
          },
          Flatten(path: "product") {
            Fetch(service: "a") {
                __typename
                id
              } =>
              {price}
            },
          },
        },
        Parallel {
          Flatten(path: "product") {
            Fetch(service: "d") {
                __typename
                isExpensiveWithDiscount
                id
              } =>
              {canAffordWithDiscount}
            },
          },
          Flatten(path: "product") {
            Fetch(service: "c") {
                __typename
                price
                id
              } =>
              {isExpensive}
            },
          },
        },
        Flatten(path: "product") {
          Fetch(service: "d") {
              __typename
              isExpensive
              id
            } =>
            {canAfford}
          },
        },
      },
    },
    "#);

    Ok(())
}

#[test]
fn two_fields_same_requirement_different_order() -> Result<(), Box<dyn Error>> {
    init_logger();
    let graph = read_supergraph("fixture/tests/requires_requires.supergraph.graphql");
    let document = parse_operation(
        r#"
        query {
          product {
            canAffordWithAndWithoutDiscount
            canAffordWithAndWithoutDiscount2
          }
        }"#,
    );
    let operation = get_operation_to_execute(&document).expect("failed to locate operation");
    let best_paths_per_leaf = walk_operation(&graph, operation)?;
    assert_eq!(best_paths_per_leaf.len(), 2);
    assert_eq!(best_paths_per_leaf[0].len(), 1);
    assert_eq!(best_paths_per_leaf[1].len(), 1);

    insta::assert_snapshot!(best_paths_per_leaf[0][0].pretty_print(&graph), @"root(Query) -(b)- Query/b -(product)- Product/b -(🔑🧩{id})- Product/d -(canAffordWithAndWithoutDiscount2🧩{isExpensive isExpensiveWithDiscount})- Boolean/d");
    insta::assert_snapshot!(best_paths_per_leaf[1][0].pretty_print(&graph), @"root(Query) -(b)- Query/b -(product)- Product/b -(🔑🧩{id})- Product/d -(canAffordWithAndWithoutDiscount🧩{isExpensive isExpensiveWithDiscount})- Boolean/d");

    let qtps = paths_to_trees(&graph, &best_paths_per_leaf);
    let query_tree = QueryTree::merge_trees(qtps);

    insta::assert_snapshot!(query_tree.pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/b)
        product of Product/b
          🧩 [
            id of ID/b
          ]
          🔑 Product/d
            🧩 [
              🧩 [
                id of ID/d
              ]
              🔑 Product/c
                🧩 [
                  🧩 [
                    id of ID/c
                  ]
                  🔑 Product/a
                    price of Float/a
                ]
                isExpensive of Boolean/c
                🧩 [
                  🧩 [
                    id of ID/c
                  ]
                  🔑 Product/b
                    hasDiscount of Boolean/b
                ]
                isExpensiveWithDiscount of Boolean/c
            ]
            canAffordWithAndWithoutDiscount2 of Boolean/d
            🧩 [
              🧩 [
                id of ID/d
              ]
              🔑 Product/c
                🧩 [
                  🧩 [
                    id of ID/c
                  ]
                  🔑 Product/a
                    price of Float/a
                ]
                isExpensive of Boolean/c
                🧩 [
                  🧩 [
                    id of ID/c
                  ]
                  🔑 Product/b
                    hasDiscount of Boolean/b
                ]
                isExpensiveWithDiscount of Boolean/c
            ]
            canAffordWithAndWithoutDiscount of Boolean/d
    ");

    let fetch_graph = build_fetch_graph_from_query_tree(&graph, query_tree)?;
    let query_plan = build_query_plan_from_fetch_graph(fetch_graph)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "b") {
          {product{__typename id hasDiscount}}
        },
        Parallel {
          Flatten(path: "product") {
            Fetch(service: "c") {
                __typename
                hasDiscount
                id
              } =>
              {isExpensiveWithDiscount}
            },
          },
          Flatten(path: "product") {
            Fetch(service: "a") {
                __typename
                id
              } =>
              {price}
            },
          },
        },
        Flatten(path: "product") {
          Fetch(service: "c") {
              __typename
              price
              id
            } =>
            {isExpensive}
          },
        },
        Flatten(path: "product") {
          Fetch(service: "d") {
              __typename
              isExpensive
              isExpensiveWithDiscount
              id
            } =>
            {canAffordWithAndWithoutDiscount canAffordWithAndWithoutDiscount2}
          },
        },
      },
    },
    "#);

    Ok(())
}

#[test]
fn many() -> Result<(), Box<dyn Error>> {
    init_logger();
    let graph = read_supergraph("fixture/tests/requires_requires.supergraph.graphql");
    let document = parse_operation(
        r#"
        query {
          product {
            id
            price
            hasDiscount
            isExpensive
            isExpensiveWithDiscount
            canAfford
            canAfford2
            canAffordWithDiscount
            canAffordWithDiscount2
          }
        }"#,
    );
    let operation = get_operation_to_execute(&document).expect("failed to locate operation");
    let best_paths_per_leaf = walk_operation(&graph, operation)?;
    assert_eq!(best_paths_per_leaf.len(), 9);

    insta::assert_snapshot!(best_paths_per_leaf[0][0].pretty_print(&graph), @"root(Query) -(b)- Query/b -(product)- Product/b -(🔑🧩{id})- Product/d -(canAffordWithDiscount2🧩{isExpensiveWithDiscount})- Boolean/d");
    insta::assert_snapshot!(best_paths_per_leaf[1][0].pretty_print(&graph), @"root(Query) -(b)- Query/b -(product)- Product/b -(🔑🧩{id})- Product/d -(canAffordWithDiscount🧩{isExpensiveWithDiscount})- Boolean/d");
    insta::assert_snapshot!(best_paths_per_leaf[2][0].pretty_print(&graph), @"root(Query) -(b)- Query/b -(product)- Product/b -(🔑🧩{id})- Product/d -(canAfford2🧩{isExpensive})- Boolean/d");
    insta::assert_snapshot!(best_paths_per_leaf[3][0].pretty_print(&graph), @"root(Query) -(b)- Query/b -(product)- Product/b -(🔑🧩{id})- Product/d -(canAfford🧩{isExpensive})- Boolean/d");
    insta::assert_snapshot!(best_paths_per_leaf[4][0].pretty_print(&graph), @"root(Query) -(b)- Query/b -(product)- Product/b -(🔑🧩{id})- Product/c -(isExpensiveWithDiscount🧩{hasDiscount})- Boolean/c");
    insta::assert_snapshot!(best_paths_per_leaf[5][0].pretty_print(&graph), @"root(Query) -(b)- Query/b -(product)- Product/b -(🔑🧩{id})- Product/c -(isExpensive🧩{price})- Boolean/c");
    insta::assert_snapshot!(best_paths_per_leaf[6][0].pretty_print(&graph), @"root(Query) -(b)- Query/b -(product)- Product/b -(hasDiscount)- Boolean/b");
    insta::assert_snapshot!(best_paths_per_leaf[7][0].pretty_print(&graph), @"root(Query) -(b)- Query/b -(product)- Product/b -(🔑🧩{id})- Product/a -(price)- Float/a");
    insta::assert_snapshot!(best_paths_per_leaf[8][0].pretty_print(&graph), @"root(Query) -(b)- Query/b -(product)- Product/b -(id)- ID/b");

    let qtps = paths_to_trees(&graph, &best_paths_per_leaf);
    let query_tree = QueryTree::merge_trees(qtps);

    insta::assert_snapshot!(query_tree.pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/b)
        product of Product/b
          🧩 [
            id of ID/b
          ]
          🔑 Product/d
            🧩 [
              🧩 [
                id of ID/d
              ]
              🔑 Product/c
                🧩 [
                  🧩 [
                    id of ID/c
                  ]
                  🔑 Product/b
                    hasDiscount of Boolean/b
                ]
                isExpensiveWithDiscount of Boolean/c
            ]
            canAffordWithDiscount2 of Boolean/d
            🧩 [
              🧩 [
                id of ID/d
              ]
              🔑 Product/c
                🧩 [
                  🧩 [
                    id of ID/c
                  ]
                  🔑 Product/b
                    hasDiscount of Boolean/b
                ]
                isExpensiveWithDiscount of Boolean/c
            ]
            canAffordWithDiscount of Boolean/d
            🧩 [
              🧩 [
                id of ID/d
              ]
              🔑 Product/c
                🧩 [
                  🧩 [
                    id of ID/c
                  ]
                  🔑 Product/a
                    price of Float/a
                ]
                isExpensive of Boolean/c
            ]
            canAfford2 of Boolean/d
            🧩 [
              🧩 [
                id of ID/d
              ]
              🔑 Product/c
                🧩 [
                  🧩 [
                    id of ID/c
                  ]
                  🔑 Product/a
                    price of Float/a
                ]
                isExpensive of Boolean/c
            ]
            canAfford of Boolean/d
          🧩 [
            id of ID/b
          ]
          🔑 Product/c
            🧩 [
              🧩 [
                id of ID/c
              ]
              🔑 Product/b
                hasDiscount of Boolean/b
            ]
            isExpensiveWithDiscount of Boolean/c
            🧩 [
              🧩 [
                id of ID/c
              ]
              🔑 Product/a
                price of Float/a
            ]
            isExpensive of Boolean/c
          hasDiscount of Boolean/b
          🧩 [
            id of ID/b
          ]
          🔑 Product/a
            price of Float/a
          id of ID/b
    ");

    let fetch_graph = build_fetch_graph_from_query_tree(&graph, query_tree)?;
    let query_plan = build_query_plan_from_fetch_graph(fetch_graph)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "b") {
          {product{__typename id hasDiscount}}
        },
        Parallel {
          Flatten(path: "product") {
            Fetch(service: "c") {
                __typename
                hasDiscount
                id
              } =>
              {isExpensiveWithDiscount}
            },
          },
          Flatten(path: "product") {
            Fetch(service: "a") {
                __typename
                id
              } =>
              {price}
            },
          },
        },
        Parallel {
          Flatten(path: "product") {
            Fetch(service: "d") {
                __typename
                isExpensiveWithDiscount
                id
              } =>
              {canAffordWithDiscount canAffordWithDiscount2}
            },
          },
          Flatten(path: "product") {
            Fetch(service: "c") {
                __typename
                price
                id
              } =>
              {isExpensive}
            },
          },
        },
        Flatten(path: "product") {
          Fetch(service: "d") {
              __typename
              isExpensive
              id
            } =>
            {canAfford canAfford2}
          },
        },
      },
    },
    "#);

    Ok(())
}
