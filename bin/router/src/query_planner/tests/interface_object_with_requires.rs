use crate::query_planner::{
    graph::PlannerOverrideContext,
    tests::testkit::{build_query_plan, build_query_plan_with_defaults, init_logger},
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
    let query_plan = build_query_plan_with_defaults(
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
    let query_plan = build_query_plan_with_defaults(
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
    let query_plan = build_query_plan_with_defaults(
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
    let query_plan = build_query_plan_with_defaults(
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
                name
                ... on User {
                  age
                  name
                  id
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
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on NodeWithName{__typename name ...on User{age name id}}}}",
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
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          {
            users {
              __typename
              ... on User {
                __typename
                age
                name
                id
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
            "operation": "{users{__typename ...on User{__typename age name id} id name}}",
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

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "a") {
        query ($v:Boolean!) {
          users {
            __typename
            ... on User {
              ... on User @skip(if: $v) {
                id
              }
            }
          }
        }
      },
    },
    "#);

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

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "a") {
        query ($a:Boolean!,$b:Boolean!) {
          users {
            __typename
            ... on User @skip(if: $a) {
              ... on User @skip(if: $b) {
                id
              }
            }
          }
        }
      },
    },
    "#);

    Ok(())
}

#[test]
fn interface_object_requires_field_external_in_one_member() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          accounts {
            fullTitle
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/interface-object-requires-external-member-field.supergraph.graphql",
        document,
    )?;

    // `fourth` needs `Account.title`. `first` has the field on the interface, but `Admin.title`
    // is `@external` there, so it can't give it for an `Admin`. That one comes from `third`.
    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "first") {
          {
            accounts {
              __typename
              id
              ... on Admin {
                __typename
                id
              }
              ... on Moderator {
                title
              }
            }
          }
        },
        Flatten(path: "accounts.@|[Admin]") {
          Fetch(service: "third") {
            {
              ... on Admin {
                __typename
                id
              }
            } =>
            {
              ... on Admin {
                title
              }
            }
          },
        },
        Flatten(path: "accounts.@") {
          Fetch(service: "fourth") {
            {
              ... on Account {
                __typename
                id
                ... on Admin {
                  title
                }
                ... on Moderator {
                  title
                }
              }
            } =>
            {
              ... on Account {
                fullTitle
              }
            }
          },
        },
      },
    },
    "#);

    Ok(())
}

/// Like `interface_object_requires_field_external_in_one_member`, with `Admin.title` moved from
/// `first` to `third` by `@override`. `first` still has the field on the interface, but not for an
/// `Admin`.
#[test]
fn interface_object_requires_field_overridden_in_one_member() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation("{ accounts { fullTitle } }");
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/interface-object-requires-overridden-member-field.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "first") {
          {
            accounts {
              __typename
              id
              ... on Admin {
                __typename
                id
              }
              ... on Moderator {
                title
              }
            }
          }
        },
        Flatten(path: "accounts.@|[Admin]") {
          Fetch(service: "third") {
            {
              ... on Admin {
                __typename
                id
              }
            } =>
            {
              ... on Admin {
                title
              }
            }
          },
        },
        Flatten(path: "accounts.@") {
          Fetch(service: "fourth") {
            {
              ... on Account {
                __typename
                id
                ... on Admin {
                  title
                }
                ... on Moderator {
                  title
                }
              }
            } =>
            {
              ... on Account {
                fullTitle
              }
            }
          },
        },
      },
    },
    "#);

    Ok(())
}

/// The same with `@override(label: "percent(50)")`: `Admin.title` comes from `third` for a
/// request in the 50%, and from `first` for the rest.
#[test]
fn interface_object_requires_field_progressively_overridden_in_one_member(
) -> Result<(), Box<dyn Error>> {
    init_logger();
    let fixture =
        "fixture/tests/interface-object-requires-progressively-overridden-member-field.supergraph.graphql";
    let document = parse_operation("{ accounts { fullTitle } }");

    let in_range = build_query_plan(
        fixture,
        document.clone(),
        PlannerOverrideContext::from_percentage(0.0),
        Default::default(),
    )?;
    insta::assert_snapshot!(format!("{}", in_range), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "first") {
          {
            accounts {
              __typename
              id
              ... on Admin {
                __typename
                id
              }
              ... on Moderator {
                title
              }
            }
          }
        },
        Flatten(path: "accounts.@|[Admin]") {
          Fetch(service: "third") {
            {
              ... on Admin {
                __typename
                id
              }
            } =>
            {
              ... on Admin {
                title
              }
            }
          },
        },
        Flatten(path: "accounts.@") {
          Fetch(service: "fourth") {
            {
              ... on Account {
                __typename
                id
                ... on Admin {
                  title
                }
                ... on Moderator {
                  title
                }
              }
            } =>
            {
              ... on Account {
                fullTitle
              }
            }
          },
        },
      },
    },
    "#);

    let out_of_range = build_query_plan(
        fixture,
        document,
        PlannerOverrideContext::from_percentage(90.0),
        Default::default(),
    )?;
    insta::assert_snapshot!(format!("{}", out_of_range), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "first") {
          {
            accounts {
              __typename
              id
              title
            }
          }
        },
        Flatten(path: "accounts.@") {
          Fetch(service: "fourth") {
            {
              ... on Account {
                __typename
                ... on Admin {
                  title
                }
                ... on Moderator {
                  title
                }
                id
              }
            } =>
            {
              ... on Account {
                fullTitle
              }
            }
          },
        },
      },
    },
    "#);

    Ok(())
}

/// The same with `@override(label: "beta")`: `Admin.title` comes from `third` when the flag is on,
/// and from `first` when it's off.
#[test]
fn interface_object_requires_field_overridden_by_flag_in_one_member() -> Result<(), Box<dyn Error>>
{
    init_logger();
    let fixture =
        "fixture/tests/interface-object-requires-flag-overridden-member-field.supergraph.graphql";
    let document = parse_operation("{ accounts { fullTitle } }");

    let on = build_query_plan(
        fixture,
        document.clone(),
        PlannerOverrideContext::from_flag("beta".to_string()),
        Default::default(),
    )?;
    insta::assert_snapshot!(format!("{}", on), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "first") {
          {
            accounts {
              __typename
              id
              ... on Admin {
                __typename
                id
              }
              ... on Moderator {
                title
              }
            }
          }
        },
        Flatten(path: "accounts.@|[Admin]") {
          Fetch(service: "third") {
            {
              ... on Admin {
                __typename
                id
              }
            } =>
            {
              ... on Admin {
                title
              }
            }
          },
        },
        Flatten(path: "accounts.@") {
          Fetch(service: "fourth") {
            {
              ... on Account {
                __typename
                id
                ... on Admin {
                  title
                }
                ... on Moderator {
                  title
                }
              }
            } =>
            {
              ... on Account {
                fullTitle
              }
            }
          },
        },
      },
    },
    "#);

    let off = build_query_plan(
        fixture,
        document,
        PlannerOverrideContext::default(),
        Default::default(),
    )?;
    insta::assert_snapshot!(format!("{}", off), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "first") {
          {
            accounts {
              __typename
              id
              title
            }
          }
        },
        Flatten(path: "accounts.@") {
          Fetch(service: "fourth") {
            {
              ... on Account {
                __typename
                ... on Admin {
                  title
                }
                ... on Moderator {
                  title
                }
                id
              }
            } =>
            {
              ... on Account {
                fullTitle
              }
            }
          },
        },
      },
    },
    "#);

    Ok(())
}

/// Asking for the field next to the one that requires it: `fourth` gets an `Admin`'s from `third`,
/// like the client does.
#[test]
fn interface_object_requires_field_progressively_overridden_and_selected(
) -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation("{ accounts { title fullTitle } }");
    let query_plan = build_query_plan(
        "fixture/tests/interface-object-requires-progressively-overridden-member-field.supergraph.graphql",
        document,
        PlannerOverrideContext::from_percentage(0.0),
        Default::default(),
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "first") {
          {
            accounts {
              __typename
              ... on Admin {
                __typename
                id
              }
              ... on Moderator {
                title
              }
              id
            }
          }
        },
        Flatten(path: "accounts.@|[Admin]") {
          Fetch(service: "third") {
            {
              ... on Admin {
                __typename
                id
              }
            } =>
            {
              ... on Admin {
                title
              }
            }
          },
        },
        Flatten(path: "accounts.@") {
          Fetch(service: "fourth") {
            {
              ... on Account {
                __typename
                id
                ... on Admin {
                  title
                }
                ... on Moderator {
                  title
                }
              }
            } =>
            {
              ... on Account {
                fullTitle
              }
            }
          },
        },
      },
    },
    "#);

    Ok(())
}
