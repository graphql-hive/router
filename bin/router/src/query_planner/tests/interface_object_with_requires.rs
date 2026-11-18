use crate::query_planner::{
    tests::testkit::{build_query_plan_with_defaults, init_logger},
    utils::parsing::parse_operation,
};
use std::error::Error;

#[test]
fn interface_object_requiring_interface_fields() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          anotherUsers {
            id
            name
            username
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/interface-object-with-requires.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    fetches
      fetch               after
      [2] b               -
      [6] a               [2]
      [4] b               [6]

    [2] b
      out
        { anotherUsers { __typename id } }

    [6] a   after [2]
      merge
        anotherUsers.@
      in
        ... on NodeWithName { __typename id }
      out
        ... on NodeWithName { name }
      rewrite in
        [NodeWithName]/__typename = "NodeWithName"

    [4] b   after [6]
      merge
        anotherUsers.@
      in
        ... on NodeWithName { __typename name id }
      out
        ... on NodeWithName { username }
      rewrite in
        [NodeWithName]/__typename = "NodeWithName"
    "#);

    insta::assert_snapshot!(format!("{}", sonic_rs::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Sequence",
        "nodes": [
          {
            "kind": "Fetch",
            "serviceName": "b",
            "operationKind": "query",
            "operation": "{anotherUsers{__typename id}}"
          },
          {
            "kind": "Flatten",
            "path": [
              {
                "Field": "anotherUsers"
              },
              "@"
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "a",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on NodeWithName{name}}}",
              "requires": [
                {
                  "kind": "InlineFragment",
                  "typeCondition": "NodeWithName",
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
              ],
              "inputRewrites": [
                {
                  "ValueSetter": {
                    "path": [
                      {
                        "TypenameEquals": [
                          "NodeWithName"
                        ]
                      },
                      {
                        "Key": "__typename"
                      }
                    ],
                    "setValueTo": "NodeWithName"
                  }
                }
              ]
            }
          },
          {
            "kind": "Flatten",
            "path": [
              {
                "Field": "anotherUsers"
              },
              "@"
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "b",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on NodeWithName{username}}}",
              "requires": [
                {
                  "kind": "InlineFragment",
                  "typeCondition": "NodeWithName",
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
              ],
              "inputRewrites": [
                {
                  "ValueSetter": {
                    "path": [
                      {
                        "TypenameEquals": [
                          "NodeWithName"
                        ]
                      },
                      {
                        "Key": "__typename"
                      }
                    ],
                    "setValueTo": "NodeWithName"
                  }
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
fn interface_field_from_remote_graph_with_requires() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          users {
            id
            name
            username
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/interface-object-with-requires.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    fetches
      fetch               after
      [2] a               -
      [4] b               [2]

    [2] a
      out
        { users { __typename id name } }
      rewrite in
        [NodeWithName]/__typename = "NodeWithName"

    [4] b   after [2]
      merge
        users.@
      in
        ... on NodeWithName { __typename name id }
      out
        ... on NodeWithName { username }
      rewrite in
        [NodeWithName]/__typename = "NodeWithName"
    "#);

    insta::assert_snapshot!(format!("{}", sonic_rs::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Sequence",
        "nodes": [
          {
            "kind": "Fetch",
            "serviceName": "a",
            "operationKind": "query",
            "operation": "{users{__typename id name}}",
            "inputRewrites": [
              {
                "ValueSetter": {
                  "path": [
                    {
                      "TypenameEquals": [
                        "NodeWithName"
                      ]
                    },
                    {
                      "Key": "__typename"
                    }
                  ],
                  "setValueTo": "NodeWithName"
                }
              }
            ]
          },
          {
            "kind": "Flatten",
            "path": [
              {
                "Field": "users"
              },
              "@"
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "b",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on NodeWithName{username}}}",
              "requires": [
                {
                  "kind": "InlineFragment",
                  "typeCondition": "NodeWithName",
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
              ],
              "inputRewrites": [
                {
                  "ValueSetter": {
                    "path": [
                      {
                        "TypenameEquals": [
                          "NodeWithName"
                        ]
                      },
                      {
                        "Key": "__typename"
                      }
                    ],
                    "setValueTo": "NodeWithName"
                  }
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
fn inline_fragment_on_interface_object_for_remote_type_field() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          anotherUsers {
            ... on User {
              age
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/interface-object-with-requires.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    fetches
      fetch               after
      [2] b               -
      [3] a               [2]

    [2] b
      out
        { anotherUsers { __typename id } }

    [3] a   after [2]
      merge
        anotherUsers.@
      in
        ... on NodeWithName { __typename id }
      out
        ... on NodeWithName { __typename ... on User { age } }
      rewrite in
        [NodeWithName]/__typename = "NodeWithName"
    "#);

    insta::assert_snapshot!(format!("{}", sonic_rs::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Sequence",
        "nodes": [
          {
            "kind": "Fetch",
            "serviceName": "b",
            "operationKind": "query",
            "operation": "{anotherUsers{__typename id}}"
          },
          {
            "kind": "Flatten",
            "path": [
              {
                "Field": "anotherUsers"
              },
              "@"
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "a",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on NodeWithName{__typename ...on User{age}}}}",
              "requires": [
                {
                  "kind": "InlineFragment",
                  "typeCondition": "NodeWithName",
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
              ],
              "inputRewrites": [
                {
                  "ValueSetter": {
                    "path": [
                      {
                        "TypenameEquals": [
                          "NodeWithName"
                        ]
                      },
                      {
                        "Key": "__typename"
                      }
                    ],
                    "setValueTo": "NodeWithName"
                  }
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
fn inline_fragment_on_local_type_behind_interface() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          users {
            ... on User {
              age
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/interface-object-with-requires.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] a               -

    [2] a
      out
        { users { __typename ... on User { age } } }
    ");

    insta::assert_snapshot!(format!("{}", sonic_rs::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Fetch",
        "serviceName": "a",
        "operationKind": "query",
        "operation": "{users{__typename ...on User{age}}}"
      }
    }
    "#);

    Ok(())
}

#[test]
fn interface_object_field_with_requires_and_inline_fragment() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          anotherUsers {
            ... on User {
              age
              id
              name
              username
            }
            id
            name
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/interface-object-with-requires.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    fetches
      fetch               after
      [2] b               -
      [6] a               [2]
      [7] b               [6]

    [2] b
      out
        { anotherUsers { __typename id } }

    [6] a   after [2]
      merge
        anotherUsers.@
      in
        ... on NodeWithName { __typename id }
      out
        ... on NodeWithName { __typename name ... on User { age name } }
      rewrite in
        [NodeWithName]/__typename = "NodeWithName"

    [7] b   after [6]
      merge
        anotherUsers.@
      in
        ... on NodeWithName { __typename name id }
      out
        ... on NodeWithName { username id }
      rewrite in
        [NodeWithName]/__typename = "NodeWithName"
    "#);

    insta::assert_snapshot!(format!("{}", sonic_rs::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Sequence",
        "nodes": [
          {
            "kind": "Fetch",
            "serviceName": "b",
            "operationKind": "query",
            "operation": "{anotherUsers{__typename id}}"
          },
          {
            "kind": "Flatten",
            "path": [
              {
                "Field": "anotherUsers"
              },
              "@"
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "a",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on NodeWithName{__typename name ...on User{age name}}}}",
              "requires": [
                {
                  "kind": "InlineFragment",
                  "typeCondition": "NodeWithName",
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
              ],
              "inputRewrites": [
                {
                  "ValueSetter": {
                    "path": [
                      {
                        "TypenameEquals": [
                          "NodeWithName"
                        ]
                      },
                      {
                        "Key": "__typename"
                      }
                    ],
                    "setValueTo": "NodeWithName"
                  }
                }
              ]
            }
          },
          {
            "kind": "Flatten",
            "path": [
              {
                "Field": "anotherUsers"
              },
              "@"
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "b",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on NodeWithName{username id}}}",
              "requires": [
                {
                  "kind": "InlineFragment",
                  "typeCondition": "NodeWithName",
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
              ],
              "inputRewrites": [
                {
                  "ValueSetter": {
                    "path": [
                      {
                        "TypenameEquals": [
                          "NodeWithName"
                        ]
                      },
                      {
                        "Key": "__typename"
                      }
                    ],
                    "setValueTo": "NodeWithName"
                  }
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
fn interface_field_from_remote_graph_with_requires_and_inline_fragment(
) -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          users {
            ... on User {
              age
              id
              name
              username
            }
            id
            name
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/interface-object-with-requires.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    fetches
      fetch               after
      [2] a               -
      [4] b               [2]

    [2] a
      out
        { users { __typename ... on User { __typename age id name } id name } }
      rewrite in
        [NodeWithName]/__typename = "NodeWithName"

    [4] b   after [2]
      merge
        users.@|[User]
      in
        ... on NodeWithName { __typename name id }
      out
        ... on NodeWithName { username }
      rewrite in
        [NodeWithName]/__typename = "NodeWithName"
    "#);

    insta::assert_snapshot!(format!("{}", sonic_rs::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Sequence",
        "nodes": [
          {
            "kind": "Fetch",
            "serviceName": "a",
            "operationKind": "query",
            "operation": "{users{__typename ...on User{__typename age id name} id name}}",
            "inputRewrites": [
              {
                "ValueSetter": {
                  "path": [
                    {
                      "TypenameEquals": [
                        "NodeWithName"
                      ]
                    },
                    {
                      "Key": "__typename"
                    }
                  ],
                  "setValueTo": "NodeWithName"
                }
              }
            ]
          },
          {
            "kind": "Flatten",
            "path": [
              {
                "Field": "users"
              },
              "@",
              {
                "TypeCondition": [
                  "User"
                ]
              }
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "b",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on NodeWithName{username}}}",
              "requires": [
                {
                  "kind": "InlineFragment",
                  "typeCondition": "NodeWithName",
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
              ],
              "inputRewrites": [
                {
                  "ValueSetter": {
                    "path": [
                      {
                        "TypenameEquals": [
                          "NodeWithName"
                        ]
                      },
                      {
                        "Key": "__typename"
                      }
                    ],
                    "setValueTo": "NodeWithName"
                  }
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

/// The same-type fragment keeps its own condition under `... on User`, with `users` being
/// `[NodeWithName]`, an interface object in another subgraph.
#[test]
fn conditional_same_type_fragment_under_a_type_condition() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query($v: Boolean!) {
          users {
            ... on User {
              ... on User @skip(if: $v) {
                id
              }
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/interface-object-with-requires.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    variables
      $v: Boolean!

    fetches
      fetch               after
      [2] a               -

    [2] a
      out
        { users { __typename ... on User { ... on User @skip(if: $v) { id } } } }
      vars
        $v
    ");

    Ok(())
}

/// Both conditions stay: `$a` on the typeless fragment, `$b` on `... on User` inside it.
#[test]
fn conditional_type_condition_inside_a_conditional_fragment() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query($a: Boolean!, $b: Boolean!) {
          users {
            ... @skip(if: $a) {
              ... on User @skip(if: $b) {
                id
              }
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/interface-object-with-requires.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    variables
      $a: Boolean!, $b: Boolean!

    fetches
      fetch               after
      [2] a               -

    [2] a
      out
        { users { __typename ... on User @skip(if: $a) { ... on User @skip(if: $b) { id } } } }
      vars
        $a, $b
    ");

    Ok(())
}
