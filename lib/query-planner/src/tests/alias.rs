use crate::{
    tests::testkit::{build_query_plan, init_logger},
    utils::parsing::parse_operation,
};
use std::error::Error;

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
    let query_plan = build_query_plan(
        "fixture/tests/circular-reference-interface.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          {
            product {
              __typename
              samePriceProduct {
                __typename
                ... on Book {
                  ...a
                }
                samePriceProduct {
                  __typename
                  ... on Book {
                    ...a
                  }
                }
              }
              ... on Book {
                __typename
                id
                samePriceProduct {
                  id
                  price
                }
              }
            }
          }
          fragment a on Book {
            id
          }
        },
        Flatten(path: "product|[Book]") {
          Fetch(service: "b") {
            {
              ... on Book {
                __typename
                id
              }
            } =>
            {
              ... on Book {
                price
              }
            }
          },
        },
      },
    },
    "#);

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
    let query_plan = build_query_plan("fixture/tests/mismatch-mix.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r###"
    QueryPlan {
      Fetch(service: "a") {
        {
          i {
            __typename
            ... on TypeA {
              strField
            }
            ... on TypeB {
              _internal_qp_alias_0: strField
            }
          }
        }
      },
    },
    "###);

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
    let query_plan = build_query_plan("fixture/tests/mismatch-mix.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r###"
    QueryPlan {
      Fetch(service: "a") {
        {
          i {
            __typename
            ... on TypeA {
              strField
              strField2
            }
            ... on TypeB {
              _internal_qp_alias_0: strField
              _internal_qp_alias_1: strField2
            }
          }
        }
      },
    },
    "###);

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
    let query_plan = build_query_plan("fixture/tests/testing.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "store") {
          {
            allProducts: products {
              __typename
              id
            }
          }
        },
        Flatten(path: "allProducts") {
          Fetch(service: "info") {
            {
              ... on Product {
                __typename
                id
              }
            } =>
            {
              ... on Product {
                available: isAvailable
                uuid
              }
            }
          },
        },
        Flatten(path: "allProducts") {
          Fetch(service: "cost") {
            {
              ... on Product {
                __typename
                uuid
              }
            } =>
            {
              ... on Product {
                price {
                  pricing: amount
                  currency
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
    let query_plan = build_query_plan(
        "fixture/tests/child-type-mismatch.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Parallel {
          Fetch(service: "b") {
            {
              accounts {
                __typename
                ... on User {
                  id
                  name
                }
                ... on Admin {
                  _internal_qp_alias_0: id
                  name
                }
              }
            }
          },
          Fetch(service: "a") {
            {
              users {
                __typename
                id
              }
            }
          },
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
              ... on User {
                name
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
    let query_plan = build_query_plan(
        "fixture/tests/child-type-mismatch.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Parallel {
          Fetch(service: "b") {
            {
              accounts {
                __typename
                ... on User {
                  id
                  name
                  similarAccounts {
                    ...a
                  }
                }
                ... on Admin {
                  _internal_qp_alias_0: id
                  name
                  similarAccounts {
                    ...a
                  }
                }
              }
            }
            fragment a on Account {
              __typename
              ... on User {
                id
                name
              }
              ... on Admin {
                _internal_qp_alias_0: id
                name
              }
            }
          },
          Fetch(service: "a") {
            {
              users {
                __typename
                id
              }
            }
          },
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
              ... on User {
                name
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
    let query_plan = build_query_plan(
        "fixture/tests/child-type-mismatch.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Parallel {
          Fetch(service: "b") {
            {
              accounts {
                __typename
                ... on User {
                  id
                  name
                  similarAccounts {
                    ...a
                  }
                }
                ... on Admin {
                  _internal_qp_alias_0: id
                  name
                  similarAccounts {
                    ...a
                  }
                }
              }
            }
            fragment a on Account {
              __typename
              ... on User {
                id
                name
                similarAccounts {
                  ...b
                }
              }
              ... on Admin {
                _internal_qp_alias_0: id
                name
                similarAccounts {
                  ...b
                }
              }
            }
            fragment b on Account {
              __typename
              ... on User {
                id
                name
              }
              ... on Admin {
                _internal_qp_alias_0: id
                name
              }
            }
          },
          Fetch(service: "a") {
            {
              users {
                __typename
                id
              }
            }
          },
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
              ... on User {
                name
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
    let query_plan = build_query_plan(
        "fixture/tests/child-type-mismatch.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "b") {
        {
          accounts {
            __typename
            ... on User {
              name
              similarAccounts {
                ...a
              }
            }
            ... on Admin {
              name
              similarAccounts {
                ...a
              }
            }
          }
        }
        fragment a on Account {
          __typename
          ... on User {
            name
            similarAccounts {
              ...b
            }
          }
          ... on Admin {
            name
            similarAccounts {
              ...b
            }
          }
        }
        fragment b on Account {
          __typename
          ... on User {
            name
          }
          ... on Admin {
            name
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
        "serviceName": "b",
        "operationKind": "query",
        "operation": "{accounts{__typename ...on User{name similarAccounts{...a}} ...on Admin{name similarAccounts{...a}}}}\n\nfragment a on Account {__typename ...on User{name similarAccounts{...b}} ...on Admin{name similarAccounts{...b}}}\nfragment b on Account {__typename ...on User{name} ...on Admin{name}}\n"
      }
    }
    "#);

    Ok(())
}
