use crate::{
    tests::testkit::{build_query_plan, init_logger},
    utils::parsing::parse_operation,
};
use std::error::Error;

/// The field the interface object resolves (`username`) is local to the root field,
/// so it's being resolved locally as well,
/// but the missing field (`name`) needs an interface entity call (interface with @key),
/// and no object types are involved. Simple.
#[test]
fn interface_object_field_local() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          # Resolves `[NodeWithName]`
          # where `NodeWithName` is an object type with @interfaceObject.
          anotherUsers {
            id
            name
            # The `username` field is in another subgraph.
            # We do not query `username` of a specific object type,
            # we query `NodeWithName.username`.
            # The `NodeWithName` is an interface type with `@key`, so
            # we're able to perform an entity call directly to NodeWithName,
            # and fetch it.
            username
          }
        }
        "#,
    );
    let query_plan = build_query_plan(
        "fixture/tests/simple-interface-object.supergraph.graphql",
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
              username
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
      },
    },
    "#);

    Ok(())
}

/// The field the interface object resolves (`username`) is not local to the root field,
/// so it's being resolved with an entity call.
/// It's similar to the `interface_object_field_local` test, but "reversed" :)
#[test]
fn interface_object_field_remote() -> Result<(), Box<dyn Error>> {
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
        "fixture/tests/simple-interface-object.supergraph.graphql",
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

    Ok(())
}

/// Query field resolves interface object type,
/// but the resolution of the missing field (`age`)
/// requires an entity call.
#[test]
fn interface_object_field_local_object_type() -> Result<(), Box<dyn Error>> {
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
        "fixture/tests/simple-interface-object.supergraph.graphql",
        document,
    )?;

    // We need to resolve User's age, but we start with `Query.anotherUsers`,
    // that resolves `[NodeWithName]`,
    // and `NodeWithName` is an object type with @interfaceObject in this subgraph.
    // It means that it cannot resolve any object types implementing the interface,
    // as it's not an interface locally (in the subgraph), it's a fake interface.
    // In this case it needs to resolve the key field,
    // and perform an entity call to the entity interface type,
    // and simply pass the query there.
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

    Ok(())
}

/// Resolves interface's implementation locally, interfaceObject is not involved.
#[test]
fn interface_to_object_type_locally() -> Result<(), Box<dyn Error>> {
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
        "fixture/tests/simple-interface-object.supergraph.graphql",
        document,
    )?;

    // In this case, the `Query.users` resolves `[NodeWithName]`,
    // that is an entity interface (has @key).
    // Because it's an interface, and not a fake inteface defined as type @interfaceObject,
    // we're able to resolve the object types implementing it.
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

    Ok(())
}

#[test]
fn interface_object_with_inline_fragment_resolving_remote_interface_field_simple(
) -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          anotherUsers {
            ... on User {
              username
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan(
        "fixture/tests/simple-interface-object.supergraph.graphql",
        document,
    )?;

    // The `Query.anotherUsers` resolves `[NodeWithName]`,
    // and in the same subgraph the `NodeWithName` is an object type with @interfaceObject.
    // It means that it's not capable of resolving interface implementations (object types),
    // as it's fake interface, an true object type.
    // To resolve the User part, we need to confirm correct __typename.
    // To resolve `User.username`, we need to call the NodeWithName interfaceObject,
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
              }
            }
          },
        },
        Flatten(path: "anotherUsers.@") {
          Fetch(service: "b") {
            {
              ... on User {
                __typename
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

    Ok(())
}

#[test]
fn interface_object_with_inline_fragment_resolving_remote_interface_field(
) -> Result<(), Box<dyn Error>> {
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
        "fixture/tests/simple-interface-object.supergraph.graphql",
        document,
    )?;

    // The `Query.anotherUsers` resolves `[NodeWithName]`,
    // and in the same subgraph the `NodeWithName` is an object type with @interfaceObject.
    // It means that it's not capable of resolving interface implementations (object types),
    // as it's fake interface, an true object type.
    // But the `id` can be resolved, they have no type condition.
    //
    // To resolve the NodeWithName.name, we need to make an entity call to the interface entity.
    //
    // To resolve the User part, we need, we need to make an entity call to the interface entity.
    //
    // To resolve `User.username`, we need to call the NodeWithName interfaceObject,
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
                }
              }
            }
          },
        },
        Flatten(path: "anotherUsers.@") {
          Fetch(service: "b") {
            {
              ... on User {
                __typename
                id
              }
            } =>
            {
              ... on NodeWithName {
                id
                username
              }
            }
          },
        },
      },
    },
    "#);

    insta::assert_snapshot!(format!("{}", serde_json::to_string_pretty(&query_plan).unwrap_or_default()), @r###"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Sequence",
        "nodes": [
          {
            "kind": "Fetch",
            "serviceName": "b",
            "operationKind": "query",
            "operation": "query{anotherUsers{__typename id}}"
          },
          {
            "kind": "Flatten",
            "path": [
              "anotherUsers",
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
                        "TypenameEquals": "NodeWithName"
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
              "anotherUsers",
              "@"
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "b",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on NodeWithName{id username}}}",
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
              ],
              "inputRewrites": [
                {
                  "ValueSetter": {
                    "path": [
                      {
                        "TypenameEquals": "NodeWithName"
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
    "###);

    Ok(())
}

#[test]
fn interface_field_with_inline_fragment_resolving_remote_interface_object_field(
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
        "fixture/tests/simple-interface-object.supergraph.graphql",
        document,
    )?;

    // The `Query.users` resolves `[NodeWithName]`,
    // and within the same subgraph, the `NodeWithName` is an interface type with `@key`.
    // That's why we can resolve the `User` part within the `Query.users` selection set.
    // The `NodeWithName.name` and `NodeWithName.id` can also be resolved locally.
    // To resolve the `User.username` we need to call the interfaceObject in another subgraph.
    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          {
            users {
              __typename
              id
              name
              ... on User {
                __typename
                age
                id
                name
              }
            }
          }
        },
        Flatten(path: "users.@") {
          Fetch(service: "b") {
            {
              ... on User {
                __typename
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

    Ok(())
}

#[test]
fn interface_object_field_local_direct_resolution() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          accounts {
            name
          }
        }
        "#,
    );
    let query_plan = build_query_plan(
        "fixture/tests/simple-interface-object.supergraph.graphql",
        document,
    )?;

    // The `Query.accounts` resolves `[Account]`,
    // and within the same subgraph it's `type Account @interfaceObject @key(fields: "id")`.
    // The interfaceObject resolves `name`.
    // Because there are no type conditions, we can resolve `Account.name` directly,
    // with no extra hops.
    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "b") {
        {
          accounts {
            name
          }
        }
      },
    },
    "#);

    Ok(())
}

#[test]
fn interface_object_field_with_inline_fragment_requiring_typename_check(
) -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          accounts {
            ... on Admin {
              name
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan(
        "fixture/tests/simple-interface-object.supergraph.graphql",
        document,
    )?;

    // The `Query.accounts` resolves `[Account]`,
    // and within the same subgraph it's `type Account @interfaceObject @key(fields: "id")`.
    // The interfaceObject resolves `name`.
    // There's a type condition, so we first need to make sure, that the result returned by `Query.accounts`,
    // is of type `Admin`.
    // That's why we collec the `id` field, and perform an entity call to the Account interface entity,
    // and resolve the `__typename`.
    // If the `__typename` equals to `Admin`, then we're able to call the interfaceObject,
    // and resolve the `name`.
    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "b") {
          {
            accounts {
              __typename
              id
            }
          }
        },
        Flatten(path: "accounts.@") {
          Fetch(service: "a") {
            {
              ... on Account {
                __typename
                id
              }
            } =>
            {
              ... on Account {
                __typename
              }
            }
          },
        },
        Flatten(path: "accounts.@") {
          Fetch(service: "b") {
            {
              ... on Admin {
                __typename
                id
              }
            } =>
            {
              ... on Account {
                name
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
fn interface_object_field_local_with_remote_typename() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          accounts {
            name
            __typename
          }
        }
        "#,
    );
    let query_plan = build_query_plan(
        "fixture/tests/simple-interface-object.supergraph.graphql",
        document,
    )?;

    // The `Query.accounts` resolves `[Account]`,
    // and the `Account` is `type @interfaceObject`.
    // The `name` field can be resolved locally,
    // as it's provided by the Account interfaceObject,
    // but the `__typename` can't.
    // It needs to represent an object type implementing the `Account` interface.
    // Because it's a fake interface type (actually object type),
    // we need to resolve it via entity call to the Accounts entity interface.
    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "b") {
          {
            accounts {
              __typename
              id
              name
            }
          }
        },
        Flatten(path: "accounts.@") {
          Fetch(service: "a") {
            {
              ... on Account {
                __typename
                id
              }
            } =>
            {
              ... on Account {
                __typename
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
fn interface_object_inline_fragment_with_remote_typename() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          accounts {
            ... on Admin {
              __typename
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan(
        "fixture/tests/simple-interface-object.supergraph.graphql",
        document,
    )?;

    // The `Query.accounts` resolves `[Account]`,
    // and it's an interface object, meaning it's a fake interface,
    // it's an object type.
    // It can't resolve `__typename` and the type condition.
    // That's why we pull `id` first.
    // We do it to perform an entity call to the Account interface entity,
    // to resolve the `__typename` there, as it's an interface,
    // and it needs to have all object types implementing Account,
    // next to it, within the subgraph.
    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "b") {
          {
            accounts {
              __typename
              id
            }
          }
        },
        Flatten(path: "accounts.@") {
          Fetch(service: "a") {
            {
              ... on Account {
                __typename
                id
              }
            } =>
            {
              ... on Account {
                __typename
                ... on Admin {
                  __typename
                }
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
fn interface_object_local_id_remote_field() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          accounts {
            id
            isActive
          }
        }
        "#,
    );
    // Same story, `Query.accounts` gives `[Account]`,
    // but the `Account` is a fake interface, it's an object type with @interfaceObject.
    // It can resolve `id`, but `isActive` lives in some other subgraph.
    // To collect it, we perform an entity call to the interface entity.
    let query_plan = build_query_plan(
        "fixture/tests/simple-interface-object.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "b") {
          {
            accounts {
              __typename
              id
            }
          }
        },
        Flatten(path: "accounts.@") {
          Fetch(service: "c") {
            {
              ... on Account {
                __typename
                id
              }
            } =>
            {
              ... on Account {
                isActive
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
fn interface_object_local_id_remote_field_with_inline_fragment() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          accounts {
            # NOTE
            # id is available in the interfaceObject and can be resolved as there's no type condition involved
            id
            ... on Admin {
              isActive
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan(
        "fixture/tests/simple-interface-object.supergraph.graphql",
        document,
    )?;

    // Similar story, `Query.accounts` gives `[Account]`,
    // but the `Account` is a fake interface, it's an object type with @interfaceObject.
    // It can resolve `id`, but `isActive` lives in some other subgraph.
    // To collect it, we perform an entity call to the interface entity,
    // and also make sure `isActive` belongs to `Admin` type.
    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "b") {
          {
            accounts {
              __typename
              id
            }
          }
        },
        Flatten(path: "accounts.@") {
          Fetch(service: "a") {
            {
              ... on Account {
                __typename
                id
              }
            } =>
            {
              ... on Account {
                __typename
                ... on Admin {
                  isActive
                }
              }
            }
          },
        },
      },
    },
    "#);

    Ok(())
}
