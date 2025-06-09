use crate::{
    ast::normalization::normalize_operation,
    planner::{
        fetch::fetch_graph::build_fetch_graph_from_query_tree,
        query_plan::build_query_plan_from_fetch_graph,
        tree::{paths_to_trees, query_tree::QueryTree},
        walker::walk_operation,
    },
    tests::testkit::{init_logger, read_supergraph},
    utils::parsing::parse_operation,
};
use std::error::Error;

// TODO: add a test that involves an entity call to fetch non-local fields from Book and Movie
//       to test how `... on X` affects the FetchGraph and FlattenNode.
#[test]
fn union_member_resolvable() -> Result<(), Box<dyn Error>> {
    init_logger();
    let (graph, consumer_schema) =
        read_supergraph("fixture/tests/union-intersection.supergraph.graphql");

    let document = parse_operation(
        r#"
        query {
          media {
            ... on Book {
              title
            }
          }
        }
        "#,
    );
    let document = normalize_operation(&consumer_schema, &document, None).unwrap();
    let operation = document.executable_operation();
    let best_paths_per_leaf = walk_operation(&graph, operation)?;
    let qtps = paths_to_trees(&graph, &best_paths_per_leaf)?;
    let query_tree = QueryTree::merge_trees(qtps);

    let fetch_graph = build_fetch_graph_from_query_tree(&graph, query_tree)?;
    let query_plan = build_query_plan_from_fetch_graph(fetch_graph)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "a") {
        {
          media {
            __typename
            ... on Book {
              title
            }
          }
        }
      },
    },
    "#);

    Ok(())
}

#[test]
fn union_member_unresolvable() -> Result<(), Box<dyn Error>> {
    init_logger();
    let (graph, consumer_schema) =
        read_supergraph("fixture/tests/union-intersection.supergraph.graphql");

    let document = parse_operation(
        r#"
        query {
          media {
            ... on Movie {
              title
            }
          }
        }
        "#,
    );
    let document = normalize_operation(&consumer_schema, &document, None).unwrap();
    let operation = document.executable_operation();
    let best_paths_per_leaf = walk_operation(&graph, operation)?;
    let qtps = paths_to_trees(&graph, &best_paths_per_leaf)?;
    let query_tree = QueryTree::merge_trees(qtps);

    let fetch_graph = build_fetch_graph_from_query_tree(&graph, query_tree)?;
    let query_plan = build_query_plan_from_fetch_graph(fetch_graph)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "a") {
        {
          media {
            __typename
          }
        }
      },
    },
    "#);

    Ok(())
}

#[test]
fn union_member_mix() -> Result<(), Box<dyn Error>> {
    init_logger();
    let (graph, consumer_schema) =
        read_supergraph("fixture/tests/union-intersection.supergraph.graphql");

    let document = parse_operation(
        r#"
        query {
          media {
            __typename
            ... on Book {
              title
            }
          }
        }
        "#,
    );
    let document = normalize_operation(&consumer_schema, &document, None).unwrap();
    let operation = document.executable_operation();
    let best_paths_per_leaf = walk_operation(&graph, operation)?;
    let qtps = paths_to_trees(&graph, &best_paths_per_leaf)?;
    let query_tree = QueryTree::merge_trees(qtps);

    let fetch_graph = build_fetch_graph_from_query_tree(&graph, query_tree)?;
    let query_plan = build_query_plan_from_fetch_graph(fetch_graph)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "a") {
        {
          media {
            __typename
            ... on Book {
              title
            }
          }
        }
      },
    },
    "#);

    Ok(())
}

#[test]
fn union_member_entity_call() -> Result<(), Box<dyn Error>> {
    init_logger();
    let (graph, consumer_schema) =
        read_supergraph("fixture/tests/union-intersection.supergraph.graphql");

    let document = parse_operation(
        r#"
        query {
          aMedia {
            __typename
            ... on Book {
              title
              aTitle
              bTitle
            }
          }
        }
        "#,
    );
    let document = normalize_operation(&consumer_schema, &document, None).unwrap();
    let operation = document.executable_operation();
    let best_paths_per_leaf = walk_operation(&graph, operation)?;
    let qtps = paths_to_trees(&graph, &best_paths_per_leaf)?;
    let query_tree = QueryTree::merge_trees(qtps);

    let fetch_graph = build_fetch_graph_from_query_tree(&graph, query_tree)?;
    let query_plan = build_query_plan_from_fetch_graph(fetch_graph)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          {
            aMedia {
              __typename
              ... on Book {
                __typename
                title
                aTitle
                id
              }
            }
          }
        },
        Flatten(path: "aMedia") {
          Fetch(service: "b") {
              ... on Book {
                __typename
                id
              }
            } =>
            {
              ... on Book {
                bTitle
              }
            }
          },
        },
      },
    },
    "#);

    Ok(())
}

#[test]
fn union_member_entity_call_many_local() -> Result<(), Box<dyn Error>> {
    init_logger();
    let (graph, consumer_schema) =
        read_supergraph("fixture/tests/union-intersection.supergraph.graphql");

    let document = parse_operation(
        r#"
        query {
          viewer {
            song {
              __typename
              ... on Song {
                title
                aTitle
              }
              ... on Movie {
                title
                bTitle
              }
              ... on Book {
                title
                aTitle
                bTitle
              }
            }
          }
        }
        "#,
    );
    let document = normalize_operation(&consumer_schema, &document, None).unwrap();
    let operation = document.executable_operation();
    let best_paths_per_leaf = walk_operation(&graph, operation)?;
    let qtps = paths_to_trees(&graph, &best_paths_per_leaf)?;
    let query_tree = QueryTree::merge_trees(qtps);

    let fetch_graph = build_fetch_graph_from_query_tree(&graph, query_tree)?;
    let query_plan = build_query_plan_from_fetch_graph(fetch_graph)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          {
            viewer {
              song {
                __typename
                ... on Book {
                  __typename
                  title
                  aTitle
                  id
                }
                ... on Song {
                  title
                  aTitle
                }
              }
            }
          }
        },
        Flatten(path: "viewer.song") {
          Fetch(service: "b") {
              ... on Book {
                __typename
                id
              }
            } =>
            {
              ... on Book {
                bTitle
              }
            }
          },
        },
      },
    },
    "#);

    Ok(())
}

#[test]
fn union_member_entity_call_many() -> Result<(), Box<dyn Error>> {
    init_logger();
    let (graph, consumer_schema) =
        read_supergraph("fixture/tests/union-intersection.supergraph.graphql");

    let document = parse_operation(
        r#"
        query {
          viewer {
            media {
              __typename
              ... on Song {
                title
                aTitle
              }
              ... on Movie {
                title
                bTitle
              }
              ... on Book {
                title
                aTitle
                bTitle
              }
            }
            book {
              __typename
              ... on Song {
                title
                aTitle
              }
              ... on Movie {
                title
                bTitle
              }
              ... on Book {
                title
                aTitle
                bTitle
              }
            }
            song {
              __typename
              ... on Song {
                title
                aTitle
              }
              ... on Movie {
                title
                bTitle
              }
              ... on Book {
                title
                aTitle
                bTitle
              }
            }
          }
        }
        "#,
    );
    let document = normalize_operation(&consumer_schema, &document, None).unwrap();
    let operation = document.executable_operation();
    let best_paths_per_leaf = walk_operation(&graph, operation)?;
    let qtps = paths_to_trees(&graph, &best_paths_per_leaf)?;
    let query_tree = QueryTree::merge_trees(qtps);

    let fetch_graph = build_fetch_graph_from_query_tree(&graph, query_tree)?;
    let query_plan = build_query_plan_from_fetch_graph(fetch_graph)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Parallel {
          Fetch(service: "b") {
            {
              viewer {
                book {
                  __typename
                  ... on Book {
                    bTitle
                  }
                }
                media {
                  __typename
                  ... on Book {
                    bTitle
                  }
                }
              }
            }
          },
          Fetch(service: "a") {
            {
              viewer {
                song {
                  __typename
                  ... on Book {
                    __typename
                    title
                    aTitle
                    id
                  }
                  ... on Song {
                    title
                    aTitle
                  }
                }
                book {
                  __typename
                  ... on Book {
                    title
                    aTitle
                  }
                }
                media {
                  __typename
                  ... on Book {
                    title
                    aTitle
                  }
                }
              }
            }
          },
        },
        Flatten(path: "viewer.song") {
          Fetch(service: "b") {
              ... on Book {
                __typename
                id
              }
            } =>
            {
              ... on Book {
                bTitle
              }
            }
          },
        },
      },
    },
    "#);

    Ok(())
}
