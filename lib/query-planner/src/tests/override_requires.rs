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
fn override_with_requires_many() -> Result<(), Box<dyn Error>> {
    init_logger();
    let graph = read_supergraph("fixture/tests/override_requires.supergraph.graphql");
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
    let document = prepare_document(document, None);
    let operation = document.executable_operation().unwrap();
    let best_paths_per_leaf = walk_operation(&graph, operation)?;
    assert_eq!(best_paths_per_leaf.len(), 12);

    insta::assert_snapshot!(best_paths_per_leaf[0][0].pretty_print(&graph), @"root(Query) -(c)- Query/c -(userInC)- User/c -(cName🧩{name})- String/c");
    insta::assert_snapshot!(best_paths_per_leaf[1][0].pretty_print(&graph), @"root(Query) -(c)- Query/c -(userInC)- User/c -(🔑🧩{id})- User/a -(aName🧩{name})- String/a");
    insta::assert_snapshot!(best_paths_per_leaf[2][0].pretty_print(&graph), @"root(Query) -(c)- Query/c -(userInC)- User/c -(🔑🧩{id})- User/b -(name)- String/b");
    insta::assert_snapshot!(best_paths_per_leaf[3][0].pretty_print(&graph), @"root(Query) -(c)- Query/c -(userInC)- User/c -(id)- ID/c");

    insta::assert_snapshot!(best_paths_per_leaf[4][0].pretty_print(&graph), @"root(Query) -(b)- Query/b -(userInB)- User/b -(🔑🧩{id})- User/c -(cName🧩{name})- String/c");
    insta::assert_snapshot!(best_paths_per_leaf[5][0].pretty_print(&graph), @"root(Query) -(b)- Query/b -(userInB)- User/b -(🔑🧩{id})- User/a -(aName🧩{name})- String/a");
    insta::assert_snapshot!(best_paths_per_leaf[6][0].pretty_print(&graph), @"root(Query) -(b)- Query/b -(userInB)- User/b -(name)- String/b");
    insta::assert_snapshot!(best_paths_per_leaf[7][0].pretty_print(&graph), @"root(Query) -(b)- Query/b -(userInB)- User/b -(id)- ID/b");

    insta::assert_snapshot!(best_paths_per_leaf[8][0].pretty_print(&graph), @"root(Query) -(a)- Query/a -(userInA)- User/a -(🔑🧩{id})- User/c -(cName🧩{name})- String/c");
    insta::assert_snapshot!(best_paths_per_leaf[9][0].pretty_print(&graph), @"root(Query) -(a)- Query/a -(userInA)- User/a -(aName🧩{name})- String/a");
    insta::assert_snapshot!(best_paths_per_leaf[10][0].pretty_print(&graph), @"root(Query) -(a)- Query/a -(userInA)- User/a -(🔑🧩{id})- User/b -(name)- String/b");
    insta::assert_snapshot!(best_paths_per_leaf[11][0].pretty_print(&graph), @"root(Query) -(a)- Query/a -(userInA)- User/a -(id)- ID/a");

    let qtps = paths_to_trees(&graph, &best_paths_per_leaf)?;

    let query_tree = QueryTree::merge_trees(qtps);
    insta::assert_snapshot!(query_tree.pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/c)
        userInC of User/c
          🧩 [
            🧩 [
              id of ID/c
            ]
            🔑 User/b
              name of String/b
          ]
          cName of String/c
          🧩 [
            id of ID/c
          ]
          🔑 User/a
            🧩 [
              🧩 [
                id of ID/a
              ]
              🔑 User/b
                name of String/b
            ]
            aName of String/a
          🧩 [
            id of ID/c
          ]
          🔑 User/b
            name of String/b
          id of ID/c
      🚪 (Query/b)
        userInB of User/b
          🧩 [
            id of ID/b
          ]
          🔑 User/c
            🧩 [
              🧩 [
                id of ID/c
              ]
              🔑 User/b
                name of String/b
            ]
            cName of String/c
          🧩 [
            id of ID/b
          ]
          🔑 User/a
            🧩 [
              🧩 [
                id of ID/a
              ]
              🔑 User/b
                name of String/b
            ]
            aName of String/a
          name of String/b
          id of ID/b
      🚪 (Query/a)
        userInA of User/a
          🧩 [
            id of ID/a
          ]
          🔑 User/c
            🧩 [
              🧩 [
                id of ID/c
              ]
              🔑 User/b
                name of String/b
            ]
            cName of String/c
          🧩 [
            🧩 [
              id of ID/a
            ]
            🔑 User/b
              name of String/b
          ]
          aName of String/a
          🧩 [
            id of ID/a
          ]
          🔑 User/b
            name of String/b
          id of ID/a
    ");

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
    let graph = read_supergraph("fixture/tests/override_requires.supergraph.graphql");
    let document = parse_operation(
        r#"
        query {
          userInC {
            cName
          }
        }"#,
    );
    let document = prepare_document(document, None);
    let operation = document.executable_operation().unwrap();
    let best_paths_per_leaf = walk_operation(&graph, operation)?;
    assert_eq!(best_paths_per_leaf.len(), 1);
    let qtps = paths_to_trees(&graph, &best_paths_per_leaf)?;

    let query_tree = QueryTree::merge_trees(qtps);
    insta::assert_snapshot!(query_tree.pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/c)
        userInC of User/c
          🧩 [
            🧩 [
              id of ID/c
            ]
            🔑 User/b
              name of String/b
          ]
          cName of String/c
    ");

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
    let graph = read_supergraph("fixture/tests/override_requires.supergraph.graphql");
    let document = parse_operation(
        r#"
        query {
          userInA {
            cName
          }
        }"#,
    );
    let document = prepare_document(document, None);
    let operation = document.executable_operation().unwrap();
    let best_paths_per_leaf = walk_operation(&graph, operation)?;
    assert_eq!(best_paths_per_leaf.len(), 1);
    let qtps = paths_to_trees(&graph, &best_paths_per_leaf)?;

    let query_tree = QueryTree::merge_trees(qtps);
    insta::assert_snapshot!(query_tree.pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/a)
        userInA of User/a
          🧩 [
            id of ID/a
          ]
          🔑 User/c
            🧩 [
              🧩 [
                id of ID/c
              ]
              🔑 User/b
                name of String/b
            ]
            cName of String/c
    ");

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
    let graph = read_supergraph("fixture/tests/override_requires.supergraph.graphql");
    let document = parse_operation(
        r#"
        query {
          userInA {
            aName
          }
        }"#,
    );
    let document = prepare_document(document, None);
    let operation = document.executable_operation().unwrap();
    let best_paths_per_leaf = walk_operation(&graph, operation)?;
    assert_eq!(best_paths_per_leaf.len(), 1);
    let qtps = paths_to_trees(&graph, &best_paths_per_leaf)?;

    let query_tree = QueryTree::merge_trees(qtps);
    insta::assert_snapshot!(query_tree.pretty_print(&graph)?, @r"
    root(Query)
      🚪 (Query/a)
        userInA of User/a
          🧩 [
            🧩 [
              id of ID/a
            ]
            🔑 User/b
              name of String/b
          ]
          aName of String/a
    ");

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
