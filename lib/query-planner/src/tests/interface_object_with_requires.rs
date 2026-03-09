use crate::{
    tests::testkit::{build_query_plan, init_logger},
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
    let query_plan = build_query_plan(
        "fixture/tests/interface-object-with-requires.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "b") {
          {
            anotherUsers {
              __typename
              id
            }
          }
        },
        Flatten(path: "anotherUsers.@") {
          Fetch(service: "a") {
            {
              ... on NodeWithName {
                __typename
                id
              }
            } =>
            {
              ... on NodeWithName {
                name
              }
            }
          },
        },
        Flatten(path: "anotherUsers.@") {
          Fetch(service: "b") {
            {
              ... on NodeWithName {
                __typename
                name
                id
              }
            } =>
            {
              ... on NodeWithName {
                username
              }
            }
          },
        },
      },
    },
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
    let query_plan = build_query_plan(
        "fixture/tests/interface-object-with-requires.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          {
            users {
              __typename
              id
              name
            }
          }
        },
        Flatten(path: "users.@") {
          Fetch(service: "b") {
            {
              ... on NodeWithName {
                __typename
                name
                id
              }
            } =>
            {
              ... on NodeWithName {
                username
              }
            }
          },
        },
      },
    },
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
    let query_plan = build_query_plan(
        "fixture/tests/interface-object-with-requires.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "b") {
          {
            anotherUsers {
              __typename
              id
            }
          }
        },
        Flatten(path: "anotherUsers.@") {
          Fetch(service: "a") {
            {
              ... on NodeWithName {
                __typename
                id
              }
            } =>
            {
              ... on NodeWithName {
                __typename
                ... on User {
                  age
                }
              }
            }
          },
        },
      },
    },
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
    let query_plan = build_query_plan(
        "fixture/tests/interface-object-with-requires.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "a") {
        {
          users {
            __typename
            ... on User {
              age
            }
          }
        }
      },
    },
    "#);

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
    let query_plan = build_query_plan(
        "fixture/tests/interface-object-with-requires.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "b") {
          {
            anotherUsers {
              __typename
              id
            }
          }
        },
        Flatten(path: "anotherUsers.@") {
          Fetch(service: "a") {
            {
              ... on NodeWithName {
                __typename
                id
              }
            } =>
            {
              ... on NodeWithName {
                __typename
                id
                name
                ... on User {
                  age
                  name
                }
              }
            }
          },
        },
        Flatten(path: "anotherUsers.@") {
          Fetch(service: "b") {
            {
              ... on NodeWithName {
                __typename
                name
                id
              }
            } =>
            {
              ... on NodeWithName {
                username
                id
              }
            }
          },
        },
      },
    },
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
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on NodeWithName{__typename id name ...on User{age name}}}}",
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
    let query_plan = build_query_plan(
        "fixture/tests/interface-object-with-requires.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          {
            users {
              __typename
              ... on User {
                __typename
                age
                id
                name
              }
              id
              name
            }
          }
        },
        Flatten(path: "users.@|[User]") {
          Fetch(service: "b") {
            {
              ... on NodeWithName {
                __typename
                name
                id
              }
            } =>
            {
              ... on NodeWithName {
                username
              }
            }
          },
        },
      },
    },
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
