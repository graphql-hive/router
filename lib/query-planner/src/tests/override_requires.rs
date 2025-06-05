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

#[test]
fn override_with_requires_many() -> Result<(), Box<dyn Error>> {
    init_logger();
    let (graph, consumer_schema) =
        read_supergraph("fixture/tests/override_requires.supergraph.graphql");
    let document = parse_operation(
        r#"
        query {
          userInA {
            id
            name
            aName
            cName
          }
          userInB {
            id
            name
            aName
            cName
          }
          userInC {
            id
            name
            aName
            cName
          }
        }"#,
    );
    let document = normalize_operation(&consumer_schema, &document, None);
    let operation = document.executable_operation();
    let best_paths_per_leaf = walk_operation(&graph, operation)?;
    assert_eq!(best_paths_per_leaf.len(), 12);
    let qtps = paths_to_trees(&graph, &best_paths_per_leaf)?;
    let query_tree = QueryTree::merge_trees(qtps);
    let fetch_graph = build_fetch_graph_from_query_tree(&graph, query_tree)?;
    let query_plan = build_query_plan_from_fetch_graph(fetch_graph)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Parallel {
          Fetch(service: "a") {
            {
              userInA {
                __typename
                id
              }
            }
          },
          Fetch(service: "b") {
            {
              userInB {
                __typename
                id
                name
              }
            }
          },
          Fetch(service: "c") {
            {
              userInC {
                __typename
                id
              }
            }
          },
        },
        Parallel {
          Flatten(path: "userInA") {
            Fetch(service: "b") {
                ... on User {
                  __typename
                  id
                }
              } =>
              {
                ... on User {
                  name
                }
              }
            },
          },
          Flatten(path: "userInB") {
            Fetch(service: "a") {
                ... on User {
                  __typename
                  name
                  id
                }
              } =>
              {
                ... on User {
                  aName
                }
              }
            },
          },
          Flatten(path: "userInB") {
            Fetch(service: "c") {
                ... on User {
                  __typename
                  name
                  id
                }
              } =>
              {
                ... on User {
                  cName
                }
              }
            },
          },
          Flatten(path: "userInC") {
            Fetch(service: "b") {
                ... on User {
                  __typename
                  id
                }
              } =>
              {
                ... on User {
                  name
                }
              }
            },
          },
        },
        Parallel {
          Flatten(path: "userInA") {
            Fetch(service: "a") {
                ... on User {
                  __typename
                  name
                  id
                }
              } =>
              {
                ... on User {
                  aName
                }
              }
            },
          },
          Flatten(path: "userInA") {
            Fetch(service: "c") {
                ... on User {
                  __typename
                  name
                  id
                }
              } =>
              {
                ... on User {
                  cName
                }
              }
            },
          },
          Flatten(path: "userInC") {
            Fetch(service: "a") {
                ... on User {
                  __typename
                  name
                  id
                }
              } =>
              {
                ... on User {
                  aName
                }
              }
            },
          },
          Flatten(path: "userInC") {
            Fetch(service: "c") {
                ... on User {
                  __typename
                  name
                  id
                }
              } =>
              {
                ... on User {
                  cName
                }
              }
            },
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
            "kind": "Parallel",
            "nodes": [
              {
                "kind": "Fetch",
                "serviceName": "a",
                "operationKind": "query",
                "operation": "{userInA{__typename id}}"
              },
              {
                "kind": "Fetch",
                "serviceName": "b",
                "operationKind": "query",
                "operation": "{userInB{__typename id name}}"
              },
              {
                "kind": "Fetch",
                "serviceName": "c",
                "operationKind": "query",
                "operation": "{userInC{__typename id}}"
              }
            ]
          },
          {
            "kind": "Parallel",
            "nodes": [
              {
                "kind": "Flatten",
                "path": [
                  "userInA"
                ],
                "node": {
                  "kind": "Fetch",
                  "serviceName": "b",
                  "operationKind": "query",
                  "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on User{name}}}",
                  "requires": [
                    {
                      "kind": "InlineFragment",
                      "typeCondition": "User",
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
                  "userInB"
                ],
                "node": {
                  "kind": "Fetch",
                  "serviceName": "a",
                  "operationKind": "query",
                  "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on User{aName}}}",
                  "requires": [
                    {
                      "kind": "InlineFragment",
                      "typeCondition": "User",
                      "selections": [
                        {
                          "kind": "Field",
                          "name": "__typename"
                        },
                        {
                          "kind": "Field",
                          "name": "name"
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
                  "userInB"
                ],
                "node": {
                  "kind": "Fetch",
                  "serviceName": "c",
                  "operationKind": "query",
                  "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on User{cName}}}",
                  "requires": [
                    {
                      "kind": "InlineFragment",
                      "typeCondition": "User",
                      "selections": [
                        {
                          "kind": "Field",
                          "name": "__typename"
                        },
                        {
                          "kind": "Field",
                          "name": "name"
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
                  "userInC"
                ],
                "node": {
                  "kind": "Fetch",
                  "serviceName": "b",
                  "operationKind": "query",
                  "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on User{name}}}",
                  "requires": [
                    {
                      "kind": "InlineFragment",
                      "typeCondition": "User",
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
            "kind": "Parallel",
            "nodes": [
              {
                "kind": "Flatten",
                "path": [
                  "userInA"
                ],
                "node": {
                  "kind": "Fetch",
                  "serviceName": "a",
                  "operationKind": "query",
                  "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on User{aName}}}",
                  "requires": [
                    {
                      "kind": "InlineFragment",
                      "typeCondition": "User",
                      "selections": [
                        {
                          "kind": "Field",
                          "name": "__typename"
                        },
                        {
                          "kind": "Field",
                          "name": "name"
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
                  "userInA"
                ],
                "node": {
                  "kind": "Fetch",
                  "serviceName": "c",
                  "operationKind": "query",
                  "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on User{cName}}}",
                  "requires": [
                    {
                      "kind": "InlineFragment",
                      "typeCondition": "User",
                      "selections": [
                        {
                          "kind": "Field",
                          "name": "__typename"
                        },
                        {
                          "kind": "Field",
                          "name": "name"
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
                  "userInC"
                ],
                "node": {
                  "kind": "Fetch",
                  "serviceName": "a",
                  "operationKind": "query",
                  "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on User{aName}}}",
                  "requires": [
                    {
                      "kind": "InlineFragment",
                      "typeCondition": "User",
                      "selections": [
                        {
                          "kind": "Field",
                          "name": "__typename"
                        },
                        {
                          "kind": "Field",
                          "name": "name"
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
                  "userInC"
                ],
                "node": {
                  "kind": "Fetch",
                  "serviceName": "c",
                  "operationKind": "query",
                  "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on User{cName}}}",
                  "requires": [
                    {
                      "kind": "InlineFragment",
                      "typeCondition": "User",
                      "selections": [
                        {
                          "kind": "Field",
                          "name": "__typename"
                        },
                        {
                          "kind": "Field",
                          "name": "name"
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
        ]
      }
    }
    "#);
    Ok(())
}

#[test]
fn override_with_requires_cname_in_c() -> Result<(), Box<dyn Error>> {
    init_logger();
    let (graph, consumer_schema) =
        read_supergraph("fixture/tests/override_requires.supergraph.graphql");
    let document = parse_operation(
        r#"
        query {
          userInC {
            cName
          }
        }"#,
    );
    let document = normalize_operation(&consumer_schema, &document, None);
    let operation = document.executable_operation();
    let best_paths_per_leaf = walk_operation(&graph, operation)?;
    assert_eq!(best_paths_per_leaf.len(), 1);
    let qtps = paths_to_trees(&graph, &best_paths_per_leaf)?;

    let query_tree = QueryTree::merge_trees(qtps);
    let fetch_graph = build_fetch_graph_from_query_tree(&graph, query_tree)?;
    let query_plan = build_query_plan_from_fetch_graph(fetch_graph)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "c") {
          {
            userInC {
              id
              __typename
            }
          }
        },
        Flatten(path: "userInC") {
          Fetch(service: "b") {
              ... on User {
                __typename
                id
              }
            } =>
            {
              ... on User {
                name
              }
            }
          },
        },
        Flatten(path: "userInC") {
          Fetch(service: "c") {
              ... on User {
                __typename
                name
                id
              }
            } =>
            {
              ... on User {
                cName
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
            "serviceName": "c",
            "operationKind": "query",
            "operation": "{userInC{id __typename}}"
          },
          {
            "kind": "Flatten",
            "path": [
              "userInC"
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "b",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on User{name}}}",
              "requires": [
                {
                  "kind": "InlineFragment",
                  "typeCondition": "User",
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
              "userInC"
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "c",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on User{cName}}}",
              "requires": [
                {
                  "kind": "InlineFragment",
                  "typeCondition": "User",
                  "selections": [
                    {
                      "kind": "Field",
                      "name": "__typename"
                    },
                    {
                      "kind": "Field",
                      "name": "name"
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
fn override_with_requires_cname_in_a() -> Result<(), Box<dyn Error>> {
    init_logger();
    let (graph, consumer_schema) =
        read_supergraph("fixture/tests/override_requires.supergraph.graphql");
    let document = parse_operation(
        r#"
        query {
          userInA {
            cName
          }
        }"#,
    );
    let document = normalize_operation(&consumer_schema, &document, None);
    let operation = document.executable_operation();
    let best_paths_per_leaf = walk_operation(&graph, operation)?;
    assert_eq!(best_paths_per_leaf.len(), 1);
    let qtps = paths_to_trees(&graph, &best_paths_per_leaf)?;
    let query_tree = QueryTree::merge_trees(qtps);
    let fetch_graph = build_fetch_graph_from_query_tree(&graph, query_tree)?;
    let query_plan = build_query_plan_from_fetch_graph(fetch_graph)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          {
            userInA {
              __typename
              id
            }
          }
        },
        Flatten(path: "userInA") {
          Fetch(service: "b") {
              ... on User {
                __typename
                id
              }
            } =>
            {
              ... on User {
                name
              }
            }
          },
        },
        Flatten(path: "userInA") {
          Fetch(service: "c") {
              ... on User {
                __typename
                name
                id
              }
            } =>
            {
              ... on User {
                cName
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
            "operation": "{userInA{__typename id}}"
          },
          {
            "kind": "Flatten",
            "path": [
              "userInA"
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "b",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on User{name}}}",
              "requires": [
                {
                  "kind": "InlineFragment",
                  "typeCondition": "User",
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
              "userInA"
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "c",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on User{cName}}}",
              "requires": [
                {
                  "kind": "InlineFragment",
                  "typeCondition": "User",
                  "selections": [
                    {
                      "kind": "Field",
                      "name": "__typename"
                    },
                    {
                      "kind": "Field",
                      "name": "name"
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
fn override_with_requires_aname_in_a() -> Result<(), Box<dyn Error>> {
    init_logger();
    let (graph, consumer_schema) =
        read_supergraph("fixture/tests/override_requires.supergraph.graphql");
    let document = parse_operation(
        r#"
        query {
          userInA {
            aName
          }
        }"#,
    );
    let document = normalize_operation(&consumer_schema, &document, None);
    let operation = document.executable_operation();
    let best_paths_per_leaf = walk_operation(&graph, operation)?;
    assert_eq!(best_paths_per_leaf.len(), 1);
    let qtps = paths_to_trees(&graph, &best_paths_per_leaf)?;
    let query_tree = QueryTree::merge_trees(qtps);
    let fetch_graph = build_fetch_graph_from_query_tree(&graph, query_tree)?;
    let query_plan = build_query_plan_from_fetch_graph(fetch_graph)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          {
            userInA {
              id
              __typename
            }
          }
        },
        Flatten(path: "userInA") {
          Fetch(service: "b") {
              ... on User {
                __typename
                id
              }
            } =>
            {
              ... on User {
                name
              }
            }
          },
        },
        Flatten(path: "userInA") {
          Fetch(service: "a") {
              ... on User {
                __typename
                name
                id
              }
            } =>
            {
              ... on User {
                aName
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
            "operation": "{userInA{id __typename}}"
          },
          {
            "kind": "Flatten",
            "path": [
              "userInA"
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "b",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on User{name}}}",
              "requires": [
                {
                  "kind": "InlineFragment",
                  "typeCondition": "User",
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
              "userInA"
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "a",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on User{aName}}}",
              "requires": [
                {
                  "kind": "InlineFragment",
                  "typeCondition": "User",
                  "selections": [
                    {
                      "kind": "Field",
                      "name": "__typename"
                    },
                    {
                      "kind": "Field",
                      "name": "name"
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
