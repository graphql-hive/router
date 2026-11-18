use crate::query_planner::{
    tests::testkit::{build_query_plan_with_defaults, init_logger},
    utils::parsing::parse_operation,
};
use std::error::Error;

/// A client alias can occupy the planner's usual internal alias name. The planner must choose
/// another wire key rather than produce duplicate response keys in the subgraph operation.
#[test]
fn client_alias_named_internal_type_mismatch() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          i {
            _internal_qp_alias_0: __typename
            ... on TypeA { strField }
            ... on TypeB { strField }
          }
        }
        "#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/tests/mismatch-mix.supergraph.graphql", document)?;

    assert!(
        !format!("{query_plan}").contains("_internal_qp_alias_0: strField"),
        "{query_plan}"
    );
    Ok(())
}

/// The internal alias collision also occurs when an entity fetch needs another argumented
/// field for `@requires`.
#[test]
fn client_alias_named_internal_requires() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        {
          things {
            _internal_qp_alias_0: id
            price(currency: "GBP")
            ... on Cat { eur }
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/requires-alias-entity-interface.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{query_plan}"), @r#"
    fetches
      fetch               after
      [2] shop            -
      [4] pricing         [2]

    [2] shop
      out
        {
          things {
            __typename
            _internal_qp_alias_0: id
            ... on Cat {
              __typename
              price(currency: "GBP")
              id
              _internal_qp_alias_1: price(currency: "EUR")
            }
            ... on Dog { price(currency: "GBP") }
          }
        }

    [4] pricing   after [2]
      merge
        things.@|[Cat]
      in
        ... on Cat { __typename price: _internal_qp_alias_1 id }
      out
        ... on Cat { eur }
    "#);
    Ok(())
}

/// The client uses `id` for a price, and the planner needs the real `id` there as the key for
/// `shop`. The key gets a key of its own, and `shop` reads it back as `id`.
#[test]
fn client_alias_named_like_a_key_field() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(r#"{ things { id: price(currency: "USD") } }"#);
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/requires-alias-entity-calls.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{query_plan}"), @r#"
    fetches
      fetch               after
      [2] catalog         -
      [3] shop            [2]

    [2] catalog
      out
        { things { __typename _internal_qp_alias_0: id } }

    [3] shop   after [2]
      merge
        things.@
      in
        ... on Thing { __typename id: _internal_qp_alias_0 }
      out
        ... on Thing { id: price(currency: "USD") }
    "#);
    Ok(())
}

/// Same as above, with the entity call behind `@include`. It merges into the root fetch, and
/// its representation has to go in as the key it's written under, not as a field of that name.
/// The `b` call for the `name` that `aName` needs goes into the `b` call for `id: name`, under
/// `... on User @include(if: $x)`, as that one goes out anyway.
#[test]
fn client_alias_named_like_a_key_field_under_a_condition() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document =
        parse_operation(r#"query ($x: Boolean!) { userInA { id: name aName @include(if: $x) } }"#);
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/override_requires.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{query_plan}"), @r"
    variables
      $x: Boolean!

    fetches
      fetch               after
      [2] a               -
      [6] b               [2]
      [4] a               [6]

    [2] a
      out
        {
          userInA {
            __typename
            _internal_qp_alias_0: id
            ... on User @include(if: $x) { __typename _internal_qp_alias_0: id }
          }
        }
      vars
        $x

    [6] b   after [2]
      merge
        userInA
      in
        ... on User { __typename id: _internal_qp_alias_0 }
      out
        ... on User { ... on User @include(if: $x) { name } id: name }
      vars
        $x

    [4] a   after [6]
      if
        $x
      merge
        userInA
      in
        ... on User { __typename name id: _internal_qp_alias_0 }
      out
        ... on User { aName }
    ");
    Ok(())
}

/// `[Item]` on `Box` and `Item` on `Bag` cannot share one response key in the subgraph
/// operation; the mismatched shape needs its own key and a response rewrite.
#[test]
fn composite_list_shape_mismatch_gets_wire_key() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          things {
            ... on Box { items { name } }
            ... on Bag { items { name } }
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/adversarial/shape-mismatch.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{query_plan}"), @r"
    fetches
      fetch               after
      [2] a               -

    [2] a
      out
        {
          things {
            __typename
            ... on Box { items { ...a } }
            ... on Bag { _internal_qp_alias_0: items { ...a } }
          }
        }
        fragment a on Item { name }
      rewrite out
        things|[Bag]/_internal_qp_alias_0 -> items
    ");
    Ok(())
}

// In this test, we can checking for a conflict that could happen on interface.
// The "samePriceProduct" field is selected on the interface, and then explicitly on "... on Book", but since it's a composite
// type, we don't have a mismatch, so no aliasing is needed.
#[test]
fn circular_reference_interface() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          product {
            __typename
            samePriceProduct {
              __typename
              ... on Book {
                id
              }
              samePriceProduct {
                __typename
                ... on Book {
                  id
                }
              }
            }
            ... on Book {
              __typename
              id
              price
              samePriceProduct {
                id
                price
              }
            }
          }
        }
"#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/circular-reference-interface.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] a               -
      [3] b               [2]

    [2] a
      out
        {
          product {
            __typename
            samePriceProduct {
              __typename
              ... on Book { ...a }
              samePriceProduct { __typename ... on Book { ...a } }
            }
            ... on Book { __typename id samePriceProduct { id price } }
          }
        }
        fragment a on Book { id }

    [3] b   after [2]
      merge
        product|[Book]
      in
        ... on Book { __typename id }
      out
        ... on Book { price }
    ");

    Ok(())
}

// In this test, we will end up with a conflict between the type of strField in TypeA and TypeB.
// This will fail in a GraphQL engine with this error:
// GraphQLError: Fields "strField" conflict because they return conflicting types "[String]" and "String". Use different aliases on the fields to fetch both if this was intentional.
#[test]
fn conflict_list_type_in_interface() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
            i {
                ... on TypeA {
                    strField # String
                }
                ... on TypeB {
                    strField # [String]
                }
            }
        }
"#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/tests/mismatch-mix.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] a               -

    [2] a
      out
        { i { __typename ... on TypeA { strField } ... on TypeB { _internal_qp_alias_0: strField } } }
      rewrite out
        i|[TypeB]/_internal_qp_alias_0 -> strField
    ");

    insta::assert_snapshot!(format!("{}", sonic_rs::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Fetch",
        "serviceName": "a",
        "operationKind": "query",
        "operation": "{i{__typename ...on TypeA{strField} ...on TypeB{_internal_qp_alias_0: strField}}}",
        "outputRewrites": [
          {
            "KeyRenamer": {
              "path": [
                {
                  "Key": "i"
                },
                {
                  "TypenameEquals": [
                    "TypeB"
                  ]
                },
                {
                  "Key": "_internal_qp_alias_0"
                }
              ],
              "renameKeyTo": "strField"
            }
          }
        ]
      }
    }
    "#);

    Ok(())
}

#[test]
fn multiple_mismtaches_same_level() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
            i {
                ... on TypeA {
                    strField # String
                    strField2 # String
                }
                ... on TypeB {
                    strField # [String]
                    strField2 # [String]
                }
            }
        }
"#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/tests/mismatch-mix.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] a               -

    [2] a
      out
        {
          i {
            __typename
            ... on TypeA { strField strField2 }
            ... on TypeB { _internal_qp_alias_0: strField _internal_qp_alias_1: strField2 }
          }
        }
      rewrite out
        i|[TypeB]/_internal_qp_alias_0 -> strField
        i|[TypeB]/_internal_qp_alias_1 -> strField2
    ");

    insta::assert_snapshot!(format!("{}", sonic_rs::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Fetch",
        "serviceName": "a",
        "operationKind": "query",
        "operation": "{i{__typename ...on TypeA{strField strField2} ...on TypeB{_internal_qp_alias_0: strField _internal_qp_alias_1: strField2}}}",
        "outputRewrites": [
          {
            "KeyRenamer": {
              "path": [
                {
                  "Key": "i"
                },
                {
                  "TypenameEquals": [
                    "TypeB"
                  ]
                },
                {
                  "Key": "_internal_qp_alias_0"
                }
              ],
              "renameKeyTo": "strField"
            }
          },
          {
            "KeyRenamer": {
              "path": [
                {
                  "Key": "i"
                },
                {
                  "TypenameEquals": [
                    "TypeB"
                  ]
                },
                {
                  "Key": "_internal_qp_alias_1"
                }
              ],
              "renameKeyTo": "strField2"
            }
          }
        ]
      }
    }
    "#);

    Ok(())
}

#[test]
fn aliasing_both_parent_and_leaf() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
            query {
              allProducts: products {
                price {
                  pricing: amount
                  currency
                }
                available: isAvailable
              }
            }"#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/tests/testing.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] store           -
      [5] info            [2]
      [3] cost            [5]

    [2] store
      out
        { allProducts: products { __typename id } }

    [5] info   after [2]
      merge
        allProducts
      in
        ... on Product { __typename id }
      out
        ... on Product { available: isAvailable uuid }

    [3] cost   after [5]
      merge
        allProducts
      in
        ... on Product { __typename uuid }
      out
        ... on Product { price { pricing: amount currency } }
    ");

    Ok(())
}

#[test]
/// Supergraph:
///   - User.id: ID (in one subgraph it's `ID`, in the other it's `ID!`)
///   - Admin.id: ID (in both subgraphs)
///
/// GraphQL has rules for nullability compatibility, and because the "final" type of User.id is nullable ID,
/// but the subgraph schema has it non-nullable, we hit an issue.
/// In the Query Planner, we need to make sure this colision is detected and prevented.
/// The cheapest solution to the problem is to alias conflicting fields and unwrap the alias from the result.
fn simple_mismatch_between_union_fields() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          users {
            id
            name
          }
          accounts {
            ... on User {
              id
              name
            }
            ... on Admin {
              id
              name
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/child-type-mismatch.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [4] b               -
      [2] a               -
      [3] b               [2]

    [4] b
      out
        {
          accounts { __typename ... on User { id name } ... on Admin { _internal_qp_alias_0: id name } }
        }
      rewrite out
        accounts|[Admin]/_internal_qp_alias_0 -> id

    [2] a
      out
        { users { __typename id } }

    [3] b   after [2]
      merge
        users.@
      in
        ... on User { __typename id }
      out
        ... on User { name }
    ");

    insta::assert_snapshot!(format!("{}", sonic_rs::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
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
                "serviceName": "b",
                "operationKind": "query",
                "operation": "{accounts{__typename ...on User{id name} ...on Admin{_internal_qp_alias_0: id name}}}",
                "outputRewrites": [
                  {
                    "KeyRenamer": {
                      "path": [
                        {
                          "Key": "accounts"
                        },
                        {
                          "TypenameEquals": [
                            "Admin"
                          ]
                        },
                        {
                          "Key": "_internal_qp_alias_0"
                        }
                      ],
                      "renameKeyTo": "id"
                    }
                  }
                ]
              },
              {
                "kind": "Fetch",
                "serviceName": "a",
                "operationKind": "query",
                "operation": "{users{__typename id}}"
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
      }
    }
    "#);

    Ok(())
}

#[test]
fn nested_internal_mismatch_between_fields() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query NestedInternalAlias {
          users {
            id
            name
          }
          accounts {
            ... on User {
              id
              name
              similarAccounts {
                ... on User {
                  id
                  name
                }
                ... on Admin {
                  id
                  name
                }
              }
            }
            ... on Admin {
              id
              name
              similarAccounts {
                ... on User {
                  id
                  name
                }
                ... on Admin {
                  id
                  name
                }
              }
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/child-type-mismatch.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [4] b               -
      [2] a               -
      [3] b               [2]

    [4] b
      out
        {
          accounts {
            __typename
            ... on User { id name similarAccounts { ...a } }
            ... on Admin { _internal_qp_alias_0: id name similarAccounts { ...a } }
          }
        }
        fragment a on Account {
          __typename
          ... on User { id name }
          ... on Admin { _internal_qp_alias_0: id name }
        }
      rewrite out
        accounts|[User]/similarAccounts|[Admin]/_internal_qp_alias_0 -> id
        accounts|[Admin]/_internal_qp_alias_0 -> id
        accounts|[Admin]/similarAccounts|[Admin]/_internal_qp_alias_0 -> id

    [2] a
      out
        { users { __typename id } }

    [3] b   after [2]
      merge
        users.@
      in
        ... on User { __typename id }
      out
        ... on User { name }
    ");

    insta::assert_snapshot!(format!("{}", sonic_rs::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
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
                "serviceName": "b",
                "operationKind": "query",
                "operation": "{accounts{__typename ...on User{id name similarAccounts{...a}} ...on Admin{_internal_qp_alias_0: id name similarAccounts{...a}}}}\n\nfragment a on Account {__typename ...on User{id name} ...on Admin{_internal_qp_alias_0: id name}}\n",
                "outputRewrites": [
                  {
                    "KeyRenamer": {
                      "path": [
                        {
                          "Key": "accounts"
                        },
                        {
                          "TypenameEquals": [
                            "User"
                          ]
                        },
                        {
                          "Key": "similarAccounts"
                        },
                        {
                          "TypenameEquals": [
                            "Admin"
                          ]
                        },
                        {
                          "Key": "_internal_qp_alias_0"
                        }
                      ],
                      "renameKeyTo": "id"
                    }
                  },
                  {
                    "KeyRenamer": {
                      "path": [
                        {
                          "Key": "accounts"
                        },
                        {
                          "TypenameEquals": [
                            "Admin"
                          ]
                        },
                        {
                          "Key": "_internal_qp_alias_0"
                        }
                      ],
                      "renameKeyTo": "id"
                    }
                  },
                  {
                    "KeyRenamer": {
                      "path": [
                        {
                          "Key": "accounts"
                        },
                        {
                          "TypenameEquals": [
                            "Admin"
                          ]
                        },
                        {
                          "Key": "similarAccounts"
                        },
                        {
                          "TypenameEquals": [
                            "Admin"
                          ]
                        },
                        {
                          "Key": "_internal_qp_alias_0"
                        }
                      ],
                      "renameKeyTo": "id"
                    }
                  }
                ]
              },
              {
                "kind": "Fetch",
                "serviceName": "a",
                "operationKind": "query",
                "operation": "{users{__typename id}}"
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
      }
    }
    "#);

    Ok(())
}

#[test]
fn deeply_nested_internal_mismatch_between_fields() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query DeeplyNestedInternalAlias {
          users {
            id
            name
          }
          accounts {
            ... on User {
              id
              name
              similarAccounts {
                ... on User {
                  id
                  name
                  similarAccounts {
                    ... on User {
                      id
                      name
                    }
                    ... on Admin {
                      id
                      name
                    }
                  }
                }
                ... on Admin {
                  id
                  name
                  similarAccounts {
                    ... on User {
                      id
                      name
                    }
                    ... on Admin {
                      id
                      name
                    }
                  }
                }
              }
            }
            ... on Admin {
              id
              name
              similarAccounts {
                ... on User {
                  id
                  name
                  similarAccounts {
                    ... on User {
                      id
                      name
                    }
                    ... on Admin {
                      id
                      name
                    }
                  }
                }
                ... on Admin {
                  id
                  name
                  similarAccounts {
                    ... on User {
                      id
                      name
                    }
                    ... on Admin {
                      id
                      name
                    }
                  }
                }
              }
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/child-type-mismatch.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [4] b               -
      [2] a               -
      [3] b               [2]

    [4] b
      out
        {
          accounts {
            __typename
            ... on User { id name similarAccounts { ...a } }
            ... on Admin { _internal_qp_alias_0: id name similarAccounts { ...a } }
          }
        }
        fragment a on Account {
          __typename
          ... on User { id name similarAccounts { ...b } }
          ... on Admin { _internal_qp_alias_0: id name similarAccounts { ...b } }
        }
        fragment b on Account {
          __typename
          ... on User { id name }
          ... on Admin { _internal_qp_alias_0: id name }
        }
      rewrite out
        accounts|[User]/similarAccounts|[User]/similarAccounts|[Admin]/_internal_qp_alias_0 -> id
        accounts|[User]/similarAccounts|[Admin]/_internal_qp_alias_0 -> id
        accounts|[User]/similarAccounts|[Admin]/similarAccounts|[Admin]/_internal_qp_alias_0 -> id
        accounts|[Admin]/_internal_qp_alias_0 -> id
        accounts|[Admin]/similarAccounts|[User]/similarAccounts|[Admin]/_internal_qp_alias_0 -> id
        accounts|[Admin]/similarAccounts|[Admin]/_internal_qp_alias_0 -> id
        accounts|[Admin]/similarAccounts|[Admin]/similarAccounts|[Admin]/_internal_qp_alias_0 -> id

    [2] a
      out
        { users { __typename id } }

    [3] b   after [2]
      merge
        users.@
      in
        ... on User { __typename id }
      out
        ... on User { name }
    ");

    insta::assert_snapshot!(format!("{}", sonic_rs::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
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
                "serviceName": "b",
                "operationKind": "query",
                "operation": "{accounts{__typename ...on User{id name similarAccounts{...a}} ...on Admin{_internal_qp_alias_0: id name similarAccounts{...a}}}}\n\nfragment a on Account {__typename ...on User{id name similarAccounts{...b}} ...on Admin{_internal_qp_alias_0: id name similarAccounts{...b}}}\nfragment b on Account {__typename ...on User{id name} ...on Admin{_internal_qp_alias_0: id name}}\n",
                "outputRewrites": [
                  {
                    "KeyRenamer": {
                      "path": [
                        {
                          "Key": "accounts"
                        },
                        {
                          "TypenameEquals": [
                            "User"
                          ]
                        },
                        {
                          "Key": "similarAccounts"
                        },
                        {
                          "TypenameEquals": [
                            "User"
                          ]
                        },
                        {
                          "Key": "similarAccounts"
                        },
                        {
                          "TypenameEquals": [
                            "Admin"
                          ]
                        },
                        {
                          "Key": "_internal_qp_alias_0"
                        }
                      ],
                      "renameKeyTo": "id"
                    }
                  },
                  {
                    "KeyRenamer": {
                      "path": [
                        {
                          "Key": "accounts"
                        },
                        {
                          "TypenameEquals": [
                            "User"
                          ]
                        },
                        {
                          "Key": "similarAccounts"
                        },
                        {
                          "TypenameEquals": [
                            "Admin"
                          ]
                        },
                        {
                          "Key": "_internal_qp_alias_0"
                        }
                      ],
                      "renameKeyTo": "id"
                    }
                  },
                  {
                    "KeyRenamer": {
                      "path": [
                        {
                          "Key": "accounts"
                        },
                        {
                          "TypenameEquals": [
                            "User"
                          ]
                        },
                        {
                          "Key": "similarAccounts"
                        },
                        {
                          "TypenameEquals": [
                            "Admin"
                          ]
                        },
                        {
                          "Key": "similarAccounts"
                        },
                        {
                          "TypenameEquals": [
                            "Admin"
                          ]
                        },
                        {
                          "Key": "_internal_qp_alias_0"
                        }
                      ],
                      "renameKeyTo": "id"
                    }
                  },
                  {
                    "KeyRenamer": {
                      "path": [
                        {
                          "Key": "accounts"
                        },
                        {
                          "TypenameEquals": [
                            "Admin"
                          ]
                        },
                        {
                          "Key": "_internal_qp_alias_0"
                        }
                      ],
                      "renameKeyTo": "id"
                    }
                  },
                  {
                    "KeyRenamer": {
                      "path": [
                        {
                          "Key": "accounts"
                        },
                        {
                          "TypenameEquals": [
                            "Admin"
                          ]
                        },
                        {
                          "Key": "similarAccounts"
                        },
                        {
                          "TypenameEquals": [
                            "User"
                          ]
                        },
                        {
                          "Key": "similarAccounts"
                        },
                        {
                          "TypenameEquals": [
                            "Admin"
                          ]
                        },
                        {
                          "Key": "_internal_qp_alias_0"
                        }
                      ],
                      "renameKeyTo": "id"
                    }
                  },
                  {
                    "KeyRenamer": {
                      "path": [
                        {
                          "Key": "accounts"
                        },
                        {
                          "TypenameEquals": [
                            "Admin"
                          ]
                        },
                        {
                          "Key": "similarAccounts"
                        },
                        {
                          "TypenameEquals": [
                            "Admin"
                          ]
                        },
                        {
                          "Key": "_internal_qp_alias_0"
                        }
                      ],
                      "renameKeyTo": "id"
                    }
                  },
                  {
                    "KeyRenamer": {
                      "path": [
                        {
                          "Key": "accounts"
                        },
                        {
                          "TypenameEquals": [
                            "Admin"
                          ]
                        },
                        {
                          "Key": "similarAccounts"
                        },
                        {
                          "TypenameEquals": [
                            "Admin"
                          ]
                        },
                        {
                          "Key": "similarAccounts"
                        },
                        {
                          "TypenameEquals": [
                            "Admin"
                          ]
                        },
                        {
                          "Key": "_internal_qp_alias_0"
                        }
                      ],
                      "renameKeyTo": "id"
                    }
                  }
                ]
              },
              {
                "kind": "Fetch",
                "serviceName": "a",
                "operationKind": "query",
                "operation": "{users{__typename id}}"
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
      }
    }
    "#);

    Ok(())
}

#[test]
fn deeply_nested_no_conflicts() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query DeeplyNested {
          accounts {
            ... on User {
              name
              similarAccounts {
                ... on User {
                  name
                  similarAccounts {
                    ... on User {
                      name
                    }
                    ... on Admin {
                      name
                    }
                  }
                }
                ... on Admin {
                  name
                  similarAccounts {
                    ... on User {
                      name
                    }
                    ... on Admin {
                      name
                    }
                  }
                }
              }
            }
            ... on Admin {
              name
              similarAccounts {
                ... on User {
                  name
                  similarAccounts {
                    ... on User {
                      name
                    }
                    ... on Admin {
                      name
                    }
                  }
                }
                ... on Admin {
                  name
                  similarAccounts {
                    ... on User {
                      name
                    }
                    ... on Admin {
                      name
                    }
                  }
                }
              }
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/child-type-mismatch.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] b               -

    [2] b
      out
        {
          accounts {
            __typename
            ... on User { name similarAccounts { ...a } }
            ... on Admin { name similarAccounts { ...a } }
          }
        }
        fragment a on Account {
          __typename
          ... on User { name similarAccounts { ...b } }
          ... on Admin { name similarAccounts { ...b } }
        }
        fragment b on Account { __typename ... on User { name } ... on Admin { name } }
    ");

    insta::assert_snapshot!(format!("{}", sonic_rs::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Fetch",
        "serviceName": "b",
        "operationKind": "query",
        "operation": "{accounts{__typename ...on User{name similarAccounts{...a}} ...on Admin{name similarAccounts{...a}}}}\n\nfragment a on Account {__typename ...on User{name similarAccounts{...b}} ...on Admin{name similarAccounts{...b}}}\nfragment b on Account {__typename ...on User{name} ...on Admin{name}}\n"
      }
    }
    "#);

    Ok(())
}

#[test]
fn multi_enum_mismatch_across_fragments() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
           query GetAllTypesWithConflictingField {
                getTypes {
                    id
                    identifier
                    code
                    metadata {
                        value
                        unit
                    }
                    ... on TypeA {
                        field # maps to EnumA
                    }
                    ... on TypeB {
                        typeBField: field # maps to EnumB
                    }
                    ... on TypeC {
                        typeCField: field # maps to EnumC
                    }
                    ... on TypeD {
                        typeDField: field # maps to EnumD
                    }
                }
            }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/mismatch-mix-enum-scalar.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] service         -

    [2] service
      out
        {
          getTypes {
            __typename
            id
            identifier
            code
            metadata { value unit }
            ... on TypeA { field }
            ... on TypeB { typeBField: field }
            ... on TypeC { typeCField: field }
            ... on TypeD { typeDField: field }
          }
        }
    ");

    insta::assert_snapshot!(format!("{}", sonic_rs::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
        {
          "kind": "QueryPlan",
          "node": {
            "kind": "Fetch",
            "serviceName": "service",
            "operationKind": "query",
            "operation": "{getTypes{__typename id identifier code metadata{value unit} ...on TypeA{field} ...on TypeB{typeBField: field} ...on TypeC{typeCField: field} ...on TypeD{typeDField: field}}}"
          }
        }
    "#);

    Ok(())
}

#[test]
fn multi_scalar_mismatch_across_fragments() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
           query GetAllTypesWithConflictingField {
                getTypes {
                    id
                    identifier
                    code
                    metadata {
                        value
                        unit
                    }
                    ... on TypeA {
                        isScalar # is a Int
                    }
                    ... on TypeB {
                        scalarB: isScalar # is a Boolean
                    }
                    ... on TypeC {
                        scalarC: isScalar # is a Float
                    }
                    ... on TypeD {
                        scalarD: isScalar # is a Int
                    }
                }
            }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/mismatch-mix-enum-scalar.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] service         -

    [2] service
      out
        {
          getTypes {
            __typename
            id
            identifier
            code
            metadata { value unit }
            ... on TypeA { isScalar }
            ... on TypeB { scalarB: isScalar }
            ... on TypeC { scalarC: isScalar }
            ... on TypeD { scalarD: isScalar }
          }
        }
    ");

    insta::assert_snapshot!(format!("{}", sonic_rs::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
        {
          "kind": "QueryPlan",
          "node": {
            "kind": "Fetch",
            "serviceName": "service",
            "operationKind": "query",
            "operation": "{getTypes{__typename id identifier code metadata{value unit} ...on TypeA{isScalar} ...on TypeB{scalarB: isScalar} ...on TypeC{scalarC: isScalar} ...on TypeD{scalarD: isScalar}}}"
          }
        }
    "#);

    Ok(())
}

#[test]
fn conflict_list_type_in_interface_preserves_client_alias() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
            i {
                ... on TypeA {
                    clientStrFieldA: strField # [String]
                }
                ... on TypeB {
                    clientStrFieldB: strField # String
                }
            }
        }
"#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/tests/mismatch-mix.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] a               -

    [2] a
      out
        {
          i {
            __typename
            ... on TypeA { clientStrFieldA: strField }
            ... on TypeB { clientStrFieldB: strField }
          }
        }
    ");

    insta::assert_snapshot!(format!("{}", sonic_rs::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Fetch",
        "serviceName": "a",
        "operationKind": "query",
        "operation": "{i{__typename ...on TypeA{clientStrFieldA: strField} ...on TypeB{clientStrFieldB: strField}}}"
      }
    }
    "#);

    Ok(())
}

#[test]
fn conflict_list_type_in_interface_with_distinct_response_keys() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
            i {
                ... on TypeA {
                    typeAField: strField # [String]
                }
                ... on TypeB {
                    typeBField: strField # String
                }
            }
        }
"#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/tests/mismatch-mix.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] a               -

    [2] a
      out
        { i { __typename ... on TypeA { typeAField: strField } ... on TypeB { typeBField: strField } } }
    ");

    insta::assert_snapshot!(format!("{}", sonic_rs::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Fetch",
        "serviceName": "a",
        "operationKind": "query",
        "operation": "{i{__typename ...on TypeA{typeAField: strField} ...on TypeB{typeBField: strField}}}"
      }
    }
    "#);

    Ok(())
}

/// `isScalar` is `Int` on `TypeA`, `Boolean` on `TypeB` and `Float` on `TypeC`. The last two
/// each need a key of their own, and not the same one, `Boolean` and `Float` can't share a key
/// either.
#[test]
fn mismatch_aliases_unique_across_sibling_fragments() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          getTypes {
            ... on TypeA { isScalar }
            ... on TypeB { isScalar }
            ... on TypeC { isScalar }
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/mismatch-mix-enum-scalar.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] service         -

    [2] service
      out
        {
          getTypes {
            __typename
            ... on TypeA { isScalar }
            ... on TypeB { _internal_qp_alias_0: isScalar }
            ... on TypeC { _internal_qp_alias_1: isScalar }
          }
        }
      rewrite out
        getTypes|[TypeB]/_internal_qp_alias_0 -> isScalar
        getTypes|[TypeC]/_internal_qp_alias_1 -> isScalar
    ");
    insta::assert_snapshot!(format!("{}", sonic_rs::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Fetch",
        "serviceName": "service",
        "operationKind": "query",
        "operation": "{getTypes{__typename ...on TypeA{isScalar} ...on TypeB{_internal_qp_alias_0: isScalar} ...on TypeC{_internal_qp_alias_1: isScalar}}}",
        "outputRewrites": [
          {
            "KeyRenamer": {
              "path": [
                {
                  "Key": "getTypes"
                },
                {
                  "TypenameEquals": [
                    "TypeB"
                  ]
                },
                {
                  "Key": "_internal_qp_alias_0"
                }
              ],
              "renameKeyTo": "isScalar"
            }
          },
          {
            "KeyRenamer": {
              "path": [
                {
                  "Key": "getTypes"
                },
                {
                  "TypenameEquals": [
                    "TypeC"
                  ]
                },
                {
                  "Key": "_internal_qp_alias_1"
                }
              ],
              "renameKeyTo": "isScalar"
            }
          }
        ]
      }
    }
    "#);

    Ok(())
}
