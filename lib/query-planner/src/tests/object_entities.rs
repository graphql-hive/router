use crate::{
    planner::{
        fetch::fetch_graph::build_fetch_graph_from_query_tree,
        query_plan::build_query_plan_from_fetch_graph,
        tree::{paths_to_trees, query_tree::QueryTree},
        walker::walk_operation,
    },
    tests::testkit::{init_logger, read_supergraph},
    utils::{operation_utils::prepare_document, parsing::parse_operation},
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
    let document = prepare_document(&document, None);
    let operation = document.executable_operation().unwrap();
    let best_paths_per_leaf = walk_operation(&graph, operation)?;
    assert_eq!(best_paths_per_leaf.len(), 3);
    let qtps = paths_to_trees(&graph, &best_paths_per_leaf)?;
    let query_tree = QueryTree::merge_trees(qtps);
    let fetch_graph = build_fetch_graph_from_query_tree(&graph, query_tree)?;
    let query_plan = build_query_plan_from_fetch_graph(fetch_graph)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "store") {
          {
            products {
              __typename
              id
            }
          }
        },
        Flatten(path: "products") {
          Fetch(service: "info") {
              ... on Product {
                __typename
                id
              }
            } =>
            {
              ... on Product {
                isAvailable
                uuid
              }
            }
          },
        },
        Flatten(path: "products") {
          Fetch(service: "cost") {
              ... on Product {
                __typename
                uuid
              }
            } =>
            {
              ... on Product {
                price {
                  currency
                  amount
                }
              }
            }
          },
        },
      },
    },
    "#);
    insta::assert_snapshot!(format!("{}", serde_json::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Sequence",
        "nodes": [
          {
            "kind": "Fetch",
            "serviceName": "store",
            "operationKind": "query",
            "operation": "{products{__typename id}}"
          },
          {
            "kind": "Flatten",
            "path": [
              "products"
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "info",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Product{isAvailable uuid}}}",
              "requires": [
                {
                  "kind": "InlineFragment",
                  "typeCondition": "Product",
                  "selections": [
                    {
                      "kind": "Field",
                      "name": "__typename"
                    },
                    {
                      "kind": "Field",
                      "name": "id"
                    }
                  ]
                }
              ]
            }
          },
          {
            "kind": "Flatten",
            "path": [
              "products"
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "cost",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Product{price{currency amount}}}}",
              "requires": [
                {
                  "kind": "InlineFragment",
                  "typeCondition": "Product",
                  "selections": [
                    {
                      "kind": "Field",
                      "name": "__typename"
                    },
                    {
                      "kind": "Field",
                      "name": "uuid"
                    }
                  ]
                }
              ]
            }
          }
        ]
      }
    }
    "#);
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
    let document = prepare_document(&document, None);
    let operation = document.executable_operation().unwrap();
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
            products {
              __typename
              id
              pid
            }
          }
        },
        Flatten(path: "products.@") {
          Fetch(service: "c") {
              ... on Product {
                __typename
                id
                pid
              }
            } =>
            {
              ... on Product {
                category {
                  details {
                    products
                  }
                }
              }
            }
          },
        },
      },
    },
    "#);
    insta::assert_snapshot!(format!("{}", serde_json::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Sequence",
        "nodes": [
          {
            "kind": "Fetch",
            "serviceName": "a",
            "operationKind": "query",
            "operation": "{products{__typename id pid}}"
          },
          {
            "kind": "Flatten",
            "path": [
              "products",
              "@"
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "c",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Product{category{details{products}}}}}",
              "requires": [
                {
                  "kind": "InlineFragment",
                  "typeCondition": "Product",
                  "selections": [
                    {
                      "kind": "Field",
                      "name": "__typename"
                    },
                    {
                      "kind": "Field",
                      "name": "id"
                    },
                    {
                      "kind": "Field",
                      "name": "pid"
                    }
                  ]
                }
              ]
            }
          }
        ]
      }
    }
    "#);
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
    let document = prepare_document(&document, None);
    let operation = document.executable_operation().unwrap();
    let best_paths_per_leaf = walk_operation(&graph, operation)?;
    assert_eq!(best_paths_per_leaf.len(), 5);
    let qtps = paths_to_trees(&graph, &best_paths_per_leaf)?;
    let query_tree = QueryTree::merge_trees(qtps);
    let fetch_graph = build_fetch_graph_from_query_tree(&graph, query_tree)?;
    let query_plan = build_query_plan_from_fetch_graph(fetch_graph)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "d") {
          {
            productFromD(id: "1") {
              __typename
              id
              name
            }
          }
        },
        Parallel {
          Flatten(path: "productFromD") {
            Fetch(service: "b") {
                ... on Product {
                  __typename
                  id
                }
              } =>
              {
                ... on Product {
                  category {
                    __typename
                    id
                  }
                }
              }
            },
          },
          Flatten(path: "productFromD") {
            Fetch(service: "a") {
                ... on Product {
                  __typename
                  id
                }
              } =>
              {
                ... on Product {
                  category {
                    details
                  }
                }
              }
            },
          },
        },
        Flatten(path: "productFromD.category") {
          Fetch(service: "c") {
              ... on Category {
                __typename
                id
              }
            } =>
            {
              ... on Category {
                name
              }
            }
          },
        },
      },
    },
    "#);
    insta::assert_snapshot!(format!("{}", serde_json::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Sequence",
        "nodes": [
          {
            "kind": "Fetch",
            "serviceName": "d",
            "operationKind": "query",
            "operation": "{productFromD(id: \"1\"){__typename id name}}"
          },
          {
            "kind": "Parallel",
            "nodes": [
              {
                "kind": "Flatten",
                "path": [
                  "productFromD"
                ],
                "node": {
                  "kind": "Fetch",
                  "serviceName": "b",
                  "operationKind": "query",
                  "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Product{category{__typename id}}}}",
                  "requires": [
                    {
                      "kind": "InlineFragment",
                      "typeCondition": "Product",
                      "selections": [
                        {
                          "kind": "Field",
                          "name": "__typename"
                        },
                        {
                          "kind": "Field",
                          "name": "id"
                        }
                      ]
                    }
                  ]
                }
              },
              {
                "kind": "Flatten",
                "path": [
                  "productFromD"
                ],
                "node": {
                  "kind": "Fetch",
                  "serviceName": "a",
                  "operationKind": "query",
                  "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Product{category{details}}}}",
                  "requires": [
                    {
                      "kind": "InlineFragment",
                      "typeCondition": "Product",
                      "selections": [
                        {
                          "kind": "Field",
                          "name": "__typename"
                        },
                        {
                          "kind": "Field",
                          "name": "id"
                        }
                      ]
                    }
                  ]
                }
              }
            ]
          },
          {
            "kind": "Flatten",
            "path": [
              "productFromD",
              "category"
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "c",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Category{name}}}",
              "requires": [
                {
                  "kind": "InlineFragment",
                  "typeCondition": "Category",
                  "selections": [
                    {
                      "kind": "Field",
                      "name": "__typename"
                    },
                    {
                      "kind": "Field",
                      "name": "id"
                    }
                  ]
                }
              ]
            }
          }
        ]
      }
    }
    "#);
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
    let document = prepare_document(&document, None);
    let operation = document.executable_operation().unwrap();
    let best_paths_per_leaf = walk_operation(&graph, operation)?;
    let qtps = paths_to_trees(&graph, &best_paths_per_leaf)?;
    let query_tree = QueryTree::merge_trees(qtps);
    let fetch_graph = build_fetch_graph_from_query_tree(&graph, query_tree)?;
    let query_plan = build_query_plan_from_fetch_graph(fetch_graph)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "products") {
          {
            topProducts {
              products {
                __typename
                id
                category {
                  tag
                  id
                }
              }
            }
          }
        },
        Flatten(path: "topProducts.products.@") {
          Fetch(service: "link") {
              ... on Product {
                __typename
                id
              }
            } =>
            {
              ... on Product {
                pid
              }
            }
          },
        },
        Flatten(path: "topProducts.products.@") {
          Fetch(service: "price") {
              ... on Product {
                __typename
                category {
                  id
                  tag
                }
                id
                pid
              }
            } =>
            {
              ... on Product {
                price {
                  price
                }
              }
            }
          },
        },
      },
    },
    "#);
    insta::assert_snapshot!(format!("{}", serde_json::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Sequence",
        "nodes": [
          {
            "kind": "Fetch",
            "serviceName": "products",
            "operationKind": "query",
            "operation": "{topProducts{products{__typename id category{tag id}}}}"
          },
          {
            "kind": "Flatten",
            "path": [
              "topProducts",
              "products",
              "@"
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "link",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Product{pid}}}",
              "requires": [
                {
                  "kind": "InlineFragment",
                  "typeCondition": "Product",
                  "selections": [
                    {
                      "kind": "Field",
                      "name": "__typename"
                    },
                    {
                      "kind": "Field",
                      "name": "id"
                    }
                  ]
                }
              ]
            }
          },
          {
            "kind": "Flatten",
            "path": [
              "topProducts",
              "products",
              "@"
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "price",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Product{price{price}}}}",
              "requires": [
                {
                  "kind": "InlineFragment",
                  "typeCondition": "Product",
                  "selections": [
                    {
                      "kind": "Field",
                      "name": "__typename"
                    },
                    {
                      "kind": "Field",
                      "name": "category",
                      "selections": [
                        {
                          "kind": "Field",
                          "name": "id"
                        },
                        {
                          "kind": "Field",
                          "name": "tag"
                        }
                      ]
                    },
                    {
                      "kind": "Field",
                      "name": "id"
                    },
                    {
                      "kind": "Field",
                      "name": "pid"
                    }
                  ]
                }
              ]
            }
          }
        ]
      }
    }
    "#);
    Ok(())
}
