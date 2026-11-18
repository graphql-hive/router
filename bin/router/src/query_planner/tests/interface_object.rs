use crate::query_planner::{
    tests::testkit::{build_query_plan_with_defaults, init_logger},
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
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/simple-interface-object.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    fetches
      fetch               after
      [2] b               -
      [3] a               [2]

    [2] b
      out
        { anotherUsers { __typename id username } }

    [3] a   after [2]
      merge
        anotherUsers.@
      in
        ... on NodeWithName { __typename id }
      out
        ... on NodeWithName { name }
      rewrite in
        [NodeWithName]/__typename = "NodeWithName"
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
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/simple-interface-object.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    fetches
      fetch               after
      [2] a               -
      [3] b               [2]

    [2] a
      out
        { users { __typename id name } }

    [3] b   after [2]
      merge
        users.@
      in
        ... on NodeWithName { __typename id }
      out
        ... on NodeWithName { username }
      rewrite in
        [NodeWithName]/__typename = "NodeWithName"
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
    let query_plan = build_query_plan_with_defaults(
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
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/simple-interface-object.supergraph.graphql",
        document,
    )?;

    // In this case, the `Query.users` resolves `[NodeWithName]`,
    // that is an entity interface (has @key).
    // Because it's an interface, and not a fake inteface defined as type @interfaceObject,
    // we're able to resolve the object types implementing it.
    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] a               -

    [2] a
      out
        { users { __typename ... on User { age } } }
    ");

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
    let query_plan = build_query_plan_with_defaults(
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
    fetches
      fetch               after
      [2] b               -
      [5] a               [2]
      [3] b               [5]

    [2] b
      out
        { anotherUsers { __typename id } }

    [5] a   after [2]
      merge
        anotherUsers.@
      in
        ... on NodeWithName { __typename id }
      out
        ... on NodeWithName { __typename }
      rewrite in
        [NodeWithName]/__typename = "NodeWithName"

    [3] b   after [5]
      merge
        anotherUsers.@
      in
        ... on User { __typename id }
      out
        ... on NodeWithName { username }
      rewrite in
        [NodeWithName]/__typename = "NodeWithName"
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
    let query_plan = build_query_plan_with_defaults(
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
        ... on NodeWithName { __typename ... on User { age name } name }
      rewrite in
        [NodeWithName]/__typename = "NodeWithName"

    [4] b   after [6]
      merge
        anotherUsers.@
      in
        ... on User { __typename id }
      out
        ... on NodeWithName { id username }
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
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on NodeWithName{__typename ...on User{age name} name}}}",
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
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/simple-interface-object.supergraph.graphql",
        document,
    )?;

    // The `Query.users` resolves `[NodeWithName]`,
    // and within the same subgraph, the `NodeWithName` is an interface type with `@key`.
    // That's why we can resolve the `User` part within the `Query.users` selection set.
    // The `NodeWithName.name` and `NodeWithName.id` can also be resolved locally.
    // To resolve the `User.username` we need to call the interfaceObject in another subgraph.
    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    fetches
      fetch               after
      [2] a               -
      [3] b               [2]

    [2] a
      out
        { users { __typename ... on User { __typename age id name } id name } }

    [3] b   after [2]
      merge
        users.@|[User]
      in
        ... on User { __typename id }
      out
        ... on NodeWithName { username }
      rewrite in
        [NodeWithName]/__typename = "NodeWithName"
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
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/simple-interface-object.supergraph.graphql",
        document,
    )?;

    // The `Query.accounts` resolves `[Account]`,
    // and within the same subgraph it's `type Account @interfaceObject @key(fields: "id")`.
    // The interfaceObject resolves `name`.
    // Because there are no type conditions, we can resolve `Account.name` directly,
    // with no extra hops.
    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] b               -

    [2] b
      out
        { accounts { name } }
    ");

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
    let query_plan = build_query_plan_with_defaults(
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
    fetches
      fetch               after
      [2] b               -
      [5] a               [2]
      [3] b               [5]

    [2] b
      out
        { accounts { __typename id } }

    [5] a   after [2]
      merge
        accounts.@
      in
        ... on Account { __typename id }
      out
        ... on Account { __typename }
      rewrite in
        [Account]/__typename = "Account"

    [3] b   after [5]
      merge
        accounts.@
      in
        ... on Admin { __typename id }
      out
        ... on Account { name }
      rewrite in
        [Account]/__typename = "Account"
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
    let query_plan = build_query_plan_with_defaults(
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
    fetches
      fetch               after
      [2] b               -
      [3] a               [2]

    [2] b
      out
        { accounts { __typename name id } }

    [3] a   after [2]
      merge
        accounts.@
      in
        ... on Account { __typename id }
      out
        ... on Account { __typename }
      rewrite in
        [Account]/__typename = "Account"
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
    let query_plan = build_query_plan_with_defaults(
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
    fetches
      fetch               after
      [2] b               -
      [3] a               [2]

    [2] b
      out
        { accounts { __typename id } }

    [3] a   after [2]
      merge
        accounts.@
      in
        ... on Account { __typename id }
      out
        ... on Account { __typename ... on Admin { __typename } }
      rewrite in
        [Account]/__typename = "Account"
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
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/simple-interface-object.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    fetches
      fetch               after
      [2] b               -
      [3] c               [2]

    [2] b
      out
        { accounts { __typename id } }

    [3] c   after [2]
      merge
        accounts.@
      in
        ... on Account { __typename id }
      out
        ... on Account { isActive }
      rewrite in
        [Account]/__typename = "Account"
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
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/simple-interface-object.supergraph.graphql",
        document,
    )?;

    // Similar story, `Query.accounts` gives `[Account]`,
    // but the `Account` is a fake interface, it's an object type with @interfaceObject.
    // It can resolve `id`, but `isActive` lives in some other subgraph.
    // To collect it, we perform an entity call to the interface entity,
    // and also make sure `isActive` belongs to `Admin` type.
    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    fetches
      fetch               after
      [2] b               -
      [3] a               [2]

    [2] b
      out
        { accounts { __typename id } }

    [3] a   after [2]
      merge
        accounts.@
      in
        ... on Account { __typename id }
      out
        ... on Account { __typename ... on Admin { isActive } }
      rewrite in
        [Account]/__typename = "Account"
    "#);

    Ok(())
}

/// `social` has `Account` as an `@interfaceObject` with `friends: [Account!]!`, and `users` has
/// the real `interface Account @key(fields: "id")` with `User`. `name` only lives on `User` in
/// `users`, so `... on User` has to go to `users` first.
///
/// Two ways lead to `... on User` from `Account/social`: one stays in `social`, the other goes to
/// `User/users`. We used to keep only the first one, and then failed with "No paths found" for
/// `name`.
#[test]
fn interface_object_list_field_with_type_condition() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        {
          me {
            friends {
              ... on User {
                name
              }
              credits
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/interface-object-nested-list.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    fetches
      fetch               after
      [2] social          -
      [3] users           [2]

    [2] social
      out
        { me { friends { __typename id credits } } }

    [3] users   after [2]
      merge
        me.friends.@
      in
        ... on Account { __typename id }
      out
        ... on Account { __typename ... on User { name } }
      rewrite in
        [Account]/__typename = "Account"
    "#);

    Ok(())
}

/// Same as `interface_object_list_field_with_type_condition`, right on the root field.
#[test]
fn interface_object_type_condition_on_root_field() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        {
          me {
            ... on User {
              name
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/interface-object-nested-list.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    fetches
      fetch               after
      [2] social          -
      [3] users           [2]

    [2] social
      out
        { me { __typename id } }

    [3] users   after [2]
      merge
        me
      in
        ... on Account { __typename id }
      out
        ... on Account { __typename ... on User { name } }
      rewrite in
        [Account]/__typename = "Account"
    "#);

    Ok(())
}

/// `users` is `[NodeWithName]`, an interface object in `b`, so `... on User` has a path through
/// `b` too, where the tail is still `NodeWithName`. The `@skip` fragment inside it has to keep its
/// condition on every path, `b`'s included.
#[test]
fn conditional_same_type_fragment_under_a_type_condition_on_an_interface_object(
) -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query($v: Boolean!) {
          users {
            ... on User {
              ... @skip(if: $v) {
                age
              }
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/simple-interface-object.supergraph.graphql",
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
        { users { __typename ... on User { ... on User @skip(if: $v) { age } } } }
      vars
        $v
    ");

    Ok(())
}

/// Same as above with `@include`, on an explicit `... on User`.
#[test]
fn included_same_type_fragment_under_a_type_condition_on_an_interface_object(
) -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query($v: Boolean!) {
          users {
            ... on User {
              ... on User @include(if: $v) {
                age
              }
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/simple-interface-object.supergraph.graphql",
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
        { users { __typename ... on User { ... on User @include(if: $v) { age } } } }
      vars
        $v
    ");

    Ok(())
}
