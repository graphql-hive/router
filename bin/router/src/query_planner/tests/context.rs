use super::testkit::{build_query_plan_from_sdl, build_query_plan_with_defaults, read_supergraph};
use crate::query_planner::utils::parsing::parse_operation;
use std::error::Error;

#[test]
fn empty_context_selection_preserves_variable_names() -> Result<(), Box<dyn Error>> {
    let supergraph = read_supergraph("fixture/tests/context-edge-cases.supergraph.graphql")
        .replace(
            "selection: \"{ currency }\"",
            "selection: \"... on Other { currency }\"",
        );
    let plan = build_query_plan_from_sdl(
        &supergraph,
        parse_operation("{ users { txs { amount } } }"),
        Default::default(),
        Default::default(),
    )?;
    let plan = sonic_rs::to_value(&plan.into_executable())?;
    let fetch = &plan["node"]["nodes"][1]["node"];
    assert_eq!(sonic_rs::to_string(&fetch["contextRewrites"])?, "[]");
    assert_eq!(
        sonic_rs::to_string(&fetch["contextVariables"])?,
        r#"["__context_0"]"#
    );
    Ok(())
}

#[test]
fn single_subgraph_nearest_ancestor() -> Result<(), Box<dyn Error>> {
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/context-basic.supergraph.graphql",
        parse_operation(r#"{ users { txs { amount } } }"#),
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          {
            users {
              __typename
              currency
              txs {
                __typename
                id
              }
            }
          }
        },
        Flatten(path: "users.@.txs.@") {
          Fetch(service: "a") {
            {
              ... on Transaction {
                __typename
                id
              }
            } =>
            ($__context_0:String) {
              ... on Transaction {
                amount(code: $__context_0)
              }
            }
          },
        },
      },
    },
    "#);
    insta::assert_json_snapshot!(query_plan, @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Sequence",
        "nodes": [
          {
            "kind": "Fetch",
            "serviceName": "a",
            "operationKind": "query",
            "operation": "{users{__typename currency txs{__typename id}}}"
          },
          {
            "kind": "Flatten",
            "path": [
              {
                "Field": "users"
              },
              "@",
              {
                "Field": "txs"
              },
              "@"
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "a",
              "variableUsages": [
                "__context_0"
              ],
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!, $__context_0:String){_entities(representations: $representations){...on Transaction{amount(code: $__context_0)}}}",
              "requires": [
                {
                  "kind": "InlineFragment",
                  "typeCondition": "Transaction",
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
              "contextVariables": [
                "__context_0"
              ],
              "contextRewrites": [
                {
                  "path": [
                    "Parent",
                    {
                      "TypenameEquals": [
                        "User"
                      ]
                    },
                    {
                      "Key": "currency"
                    }
                  ],
                  "renameKeyTo": "__context_0"
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
fn nested_ancestor_picks_nearest() -> Result<(), Box<dyn Error>> {
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/context-basic.supergraph.graphql",
        parse_operation(r#"{ users { child { txs { amount } } } }"#),
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          {
            users {
              child {
                __typename
                currency
                txs {
                  __typename
                  id
                }
              }
            }
          }
        },
        Flatten(path: "users.@.child.txs.@") {
          Fetch(service: "a") {
            {
              ... on Transaction {
                __typename
                id
              }
            } =>
            ($__context_0:String) {
              ... on Transaction {
                amount(code: $__context_0)
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
fn ancestor_through_non_context_wrapper() -> Result<(), Box<dyn Error>> {
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/context-basic.supergraph.graphql",
        parse_operation(r#"{ users { wrapper { txs { amount } } } }"#),
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          {
            users {
              __typename
              currency
              wrapper {
                txs {
                  __typename
                  id
                }
              }
            }
          }
        },
        Flatten(path: "users.@.wrapper.txs.@") {
          Fetch(service: "a") {
            {
              ... on Transaction {
                __typename
                id
              }
            } =>
            ($__context_0:String) {
              ... on Transaction {
                amount(code: $__context_0)
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
fn interface_context() -> Result<(), Box<dyn Error>> {
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/context-basic.supergraph.graphql",
        parse_operation(r#"{ account { txs { fromInterface } } }"#),
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          {
            account {
              __typename
              currency
              txs {
                __typename
                id
              }
            }
          }
        },
        Flatten(path: "account.txs.@") {
          Fetch(service: "a") {
            {
              ... on Transaction {
                __typename
                id
              }
            } =>
            ($__context_0:String) {
              ... on Transaction {
                fromInterface(code: $__context_0)
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
fn union_context_with_type_conditions() -> Result<(), Box<dyn Error>> {
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/context-basic.supergraph.graphql",
        parse_operation(
            r#"{ viewer { ... on User { txs { fromUnion } } ... on Other { txs { fromUnion } } } }"#,
        ),
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          {
            viewer {
              __typename
              ... on User {
                __typename
                currency
                txs {
                  ...a
                }
              }
              ... on Other {
                __typename
                currency
                txs {
                  ...a
                }
              }
            }
          }
          fragment a on Transaction {
            __typename
            id
          }
        },
        BatchFetch(service: "a") {
          {
            _e0 {
              paths: [
                "viewer|[Other].txs.@"
              ]
              {
                ... on Transaction {
                  __typename
                  id
                }
              }
            }
            _e1 {
              paths: [
                "viewer|[User].txs.@"
              ]
              {
                ... on Transaction {
                  __typename
                  id
                }
              }
            }
          }
          ($__context_1:String,$__context_0:String) {
            _e0: _entities(representations: $__batch_reps_0) {
              ... on Transaction {
                fromUnion(code: $__context_1)
              }
            }
            _e1: _entities(representations: $__batch_reps_1) {
              ... on Transaction {
                fromUnion(code: $__context_0)
              }
            }
          }
        },
      },
    },
    "#);
    insta::assert_json_snapshot!(query_plan, @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Sequence",
        "nodes": [
          {
            "kind": "Fetch",
            "serviceName": "a",
            "operationKind": "query",
            "operation": "{viewer{__typename ...on User{__typename currency txs{...a}} ...on Other{__typename currency txs{...a}}}}\n\nfragment a on Transaction {__typename id}\n"
          },
          {
            "kind": "BatchFetch",
            "serviceName": "a",
            "variableUsages": [
              "__context_0",
              "__context_1"
            ],
            "operationKind": "query",
            "operation": "query($__batch_reps_0:[_Any!]!, $__batch_reps_1:[_Any!]!, $__context_1:String, $__context_0:String){_e0: _entities(representations: $__batch_reps_0){...on Transaction{fromUnion(code: $__context_1)}} _e1: _entities(representations: $__batch_reps_1){...on Transaction{fromUnion(code: $__context_0)}}}",
            "entityBatch": {
              "aliases": [
                {
                  "alias": "_e0",
                  "representationsVariableName": "__batch_reps_0",
                  "paths": [
                    [
                      {
                        "Field": "viewer"
                      },
                      {
                        "TypeCondition": [
                          "Other"
                        ]
                      },
                      {
                        "Field": "txs"
                      },
                      "@"
                    ]
                  ],
                  "requires": [
                    {
                      "kind": "InlineFragment",
                      "typeCondition": "Transaction",
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
                  "contextRewrites": [
                    {
                      "path": [
                        "Parent",
                        {
                          "TypenameEquals": [
                            "User"
                          ]
                        },
                        {
                          "Key": "currency"
                        }
                      ],
                      "renameKeyTo": "__context_1"
                    },
                    {
                      "path": [
                        "Parent",
                        {
                          "TypenameEquals": [
                            "Other"
                          ]
                        },
                        {
                          "Key": "currency"
                        }
                      ],
                      "renameKeyTo": "__context_1"
                    }
                  ],
                  "contextVariables": [
                    "__context_1"
                  ]
                },
                {
                  "alias": "_e1",
                  "representationsVariableName": "__batch_reps_1",
                  "paths": [
                    [
                      {
                        "Field": "viewer"
                      },
                      {
                        "TypeCondition": [
                          "User"
                        ]
                      },
                      {
                        "Field": "txs"
                      },
                      "@"
                    ]
                  ],
                  "requires": [
                    {
                      "kind": "InlineFragment",
                      "typeCondition": "Transaction",
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
                  "contextRewrites": [
                    {
                      "path": [
                        "Parent",
                        {
                          "TypenameEquals": [
                            "User"
                          ]
                        },
                        {
                          "Key": "currency"
                        }
                      ],
                      "renameKeyTo": "__context_0"
                    },
                    {
                      "path": [
                        "Parent",
                        {
                          "TypenameEquals": [
                            "Other"
                          ]
                        },
                        {
                          "Key": "currency"
                        }
                      ],
                      "renameKeyTo": "__context_0"
                    }
                  ],
                  "contextVariables": [
                    "__context_0"
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
fn multiple_arguments_from_different_ancestors() -> Result<(), Box<dyn Error>> {
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/context-basic.supergraph.graphql",
        parse_operation(r#"{ outer { user { txs { multiple } } } }"#),
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          {
            outer {
              __typename
              code
              user {
                __typename
                currency
                txs {
                  __typename
                  id
                }
              }
            }
          }
        },
        Flatten(path: "outer.user.txs.@") {
          Fetch(service: "a") {
            {
              ... on Transaction {
                __typename
                id
              }
            } =>
            ($__context_0:String,$__context_1:String) {
              ... on Transaction {
                multiple(code: $__context_0, outer: $__context_1)
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
fn requires_and_context_on_one_field() -> Result<(), Box<dyn Error>> {
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/context-basic.supergraph.graphql",
        parse_operation(r#"{ users { txs { required } } }"#),
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          {
            users {
              __typename
              currency
              txs {
                __typename
                id
              }
            }
          }
        },
        Flatten(path: "users.@.txs.@") {
          Fetch(service: "b") {
            {
              ... on Transaction {
                __typename
                id
              }
            } =>
            {
              ... on Transaction {
                base
              }
            }
          },
        },
        Flatten(path: "users.@.txs.@") {
          Fetch(service: "a") {
            {
              ... on Transaction {
                __typename
                base
                id
              }
            } =>
            ($__context_0:String) {
              ... on Transaction {
                required(code: $__context_0)
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
fn context_field_under_include() -> Result<(), Box<dyn Error>> {
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/context-basic.supergraph.graphql",
        parse_operation(
            r#"query($include: Boolean!) { users { txs { amount @include(if: $include) } } }"#,
        ),
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          {
            users {
              __typename
              currency
              txs {
                __typename
                id
              }
            }
          }
        },
        Include(if: $include) {
          Flatten(path: "users.@.txs.@") {
            Fetch(service: "a") {
              {
                ... on Transaction {
                  __typename
                  id
                }
              } =>
              ($__context_0:String) {
                ... on Transaction {
                  amount(code: $__context_0)
                }
              }
            },
          },
        },
      },
    },
    "#);

    Ok(())
}

#[test]
fn client_alias_colliding_with_context_key() -> Result<(), Box<dyn Error>> {
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/context-basic.supergraph.graphql",
        parse_operation(r#"{ users { currency: id txs { amount } } }"#),
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          {
            users {
              __typename
              currency: id
              _internal_qp_alias_0: currency
              txs {
                __typename
                id
              }
            }
          }
        },
        Flatten(path: "users.@.txs.@") {
          Fetch(service: "a") {
            {
              ... on Transaction {
                __typename
                id
              }
            } =>
            ($__context_0:String) {
              ... on Transaction {
                amount(code: $__context_0)
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
fn two_root_fields_batch_into_one_fetch() -> Result<(), Box<dyn Error>> {
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/context-basic.supergraph.graphql",
        parse_operation(r#"{ users { txs { amount } } secondUsers { txs { amount } } }"#),
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          {
            users {
              ...a
            }
            secondUsers {
              ...a
            }
          }
          fragment a on User {
            __typename
            currency
            txs {
              __typename
              id
            }
          }
        },
        BatchFetch(service: "a") {
          {
            _e0 {
              paths: [
                "secondUsers.@.txs.@"
              ]
              {
                ... on Transaction {
                  __typename
                  id
                }
              }
            }
            _e1 {
              paths: [
                "users.@.txs.@"
              ]
              {
                ... on Transaction {
                  __typename
                  id
                }
              }
            }
          }
          ($__context_1:String,$__context_0:String) {
            _e0: _entities(representations: $__batch_reps_0) {
              ... on Transaction {
                amount(code: $__context_1)
              }
            }
            _e1: _entities(representations: $__batch_reps_1) {
              ... on Transaction {
                amount(code: $__context_0)
              }
            }
          }
        },
      },
    },
    "#);

    Ok(())
}

#[test]
fn non_null_context_argument_is_not_batched() -> Result<(), Box<dyn Error>> {
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/context-basic.supergraph.graphql",
        parse_operation(r#"{ users { txs { strict } } secondUsers { txs { strict } } }"#),
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          {
            users {
              ...a
            }
            secondUsers {
              ...a
            }
          }
          fragment a on User {
            __typename
            currency
            txs {
              __typename
              id
            }
          }
        },
        Parallel {
          Flatten(path: "secondUsers.@.txs.@") {
            Fetch(service: "a") {
              {
                ... on Transaction {
                  __typename
                  id
                }
              } =>
              ($__context_1:String!) {
                ... on Transaction {
                  strict(code: $__context_1)
                }
              }
            },
          },
          Flatten(path: "users.@.txs.@") {
            Fetch(service: "a") {
              {
                ... on Transaction {
                  __typename
                  id
                }
              } =>
              ($__context_0:String!) {
                ... on Transaction {
                  strict(code: $__context_0)
                }
              }
            },
          },
        },
      },
    },
    "#);

    Ok(())
}

#[test]
fn context_field_under_skip() -> Result<(), Box<dyn Error>> {
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/context-basic.supergraph.graphql",
        parse_operation(r#"query($skip: Boolean!) { users { txs { amount @skip(if: $skip) } } }"#),
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          {
            users {
              __typename
              currency
              txs {
                __typename
                id
              }
            }
          }
        },
        Skip(if: $skip) {
          Flatten(path: "users.@.txs.@") {
            Fetch(service: "a") {
              {
                ... on Transaction {
                  __typename
                  id
                }
              } =>
              ($__context_0:String) {
                ... on Transaction {
                  amount(code: $__context_0)
                }
              }
            },
          },
        },
      },
    },
    "#);

    Ok(())
}

#[test]
fn client_alias_on_ancestor_field() -> Result<(), Box<dyn Error>> {
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/context-basic.supergraph.graphql",
        parse_operation(r#"{ renamed: users { txs { amount } } }"#),
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          {
            renamed: users {
              __typename
              currency
              txs {
                __typename
                id
              }
            }
          }
        },
        Flatten(path: "renamed.@.txs.@") {
          Fetch(service: "a") {
            {
              ... on Transaction {
                __typename
                id
              }
            } =>
            ($__context_0:String) {
              ... on Transaction {
                amount(code: $__context_0)
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
fn cross_subgraph_merges_into_two_fetches() -> Result<(), Box<dyn Error>> {
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/context-cross.supergraph.graphql",
        parse_operation(r#"{ user { lastTransaction { amountInUserCurrency } } }"#),
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          {
            user {
              __typename
              id
              userCurrency {
                isoCode
              }
            }
          }
        },
        Flatten(path: "user") {
          Fetch(service: "b") {
            {
              ... on User {
                __typename
                id
              }
            } =>
            ($__context_0:String) {
              ... on User {
                __typename
                id
                lastTransaction {
                  __typename
                  id
                  amountInUserCurrency(currencyCode: $__context_0)
                }
              }
            }
          },
        },
      },
    },
    "#);
    insta::assert_json_snapshot!(query_plan, @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Sequence",
        "nodes": [
          {
            "kind": "Fetch",
            "serviceName": "a",
            "operationKind": "query",
            "operation": "{user{__typename id userCurrency{isoCode}}}"
          },
          {
            "kind": "Flatten",
            "path": [
              {
                "Field": "user"
              }
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "b",
              "variableUsages": [
                "__context_0"
              ],
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!, $__context_0:String){_entities(representations: $representations){...on User{__typename id lastTransaction{__typename id amountInUserCurrency(currencyCode: $__context_0)}}}}",
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
              "contextVariables": [
                "__context_0"
              ],
              "contextRewrites": [
                {
                  "path": [
                    {
                      "TypenameEquals": [
                        "User"
                      ]
                    },
                    {
                      "Key": "userCurrency"
                    },
                    {
                      "Key": "isoCode"
                    }
                  ],
                  "renameKeyTo": "__context_0"
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
fn context_in_requires_selection_is_unsupported() {
    // inline, not a fixture, so `generated` doesn't plan random operations against it
    let supergraph = r#"
        enum join__Graph {
            A @join__graph(name: "a", url: "")
            B @join__graph(name: "b", url: "")
        }
        type Query @join__type(graph: A) @join__type(graph: B) {
            outer: Outer @join__field(graph: A)
        }
        type Outer @join__type(graph: A) @context(name: "a__outer") {
            code: String
            user: User
        }
        type User @join__type(graph: A, key: "id") @join__type(graph: B, key: "id") {
            id: ID!
            txs: [Transaction] @join__field(graph: A) @join__field(graph: B, external: true)
            total: Int @join__field(graph: B, requires: "txs { amount }")
        }
        type Transaction @join__type(graph: A, key: "id") @join__type(graph: B, key: "id") {
            id: ID!
            amount: Int @join__field(graph: A, contextArguments: [
                {context: "a__outer", name: "code", type: "String", selection: "{ code }"}
            ]) @join__field(graph: B, external: true)
        }
    "#;
    let error = build_query_plan_from_sdl(
        supergraph,
        parse_operation("{ outer { user { total } } }"),
        Default::default(),
        Default::default(),
    )
    .unwrap_err();

    insta::assert_snapshot!(error, @"@fromContext on a field selected by @requires or another @fromContext selection is not supported");
}

#[test]
fn context_is_unsatisfiable_without_an_ancestor_or_a_resolvable_selection() {
    for query in [
        // no ancestor with the context
        "{ transaction { amount } }",
        // nearest ancestor can't resolve the selection
        "{ broken { txs { amount } } }",
        // no fallback to a farther ancestor
        "{ users { broken { txs { amount } } } }",
    ] {
        let error = build_query_plan_with_defaults(
            "fixture/tests/context-edge-cases.supergraph.graphql",
            parse_operation(query),
        )
        .unwrap_err();

        insta::allow_duplicates! {
            insta::assert_snapshot!(error, @"No paths found for selection item: amount");
        }
    }
}

#[test]
fn context_requires_a_locally_resolvable_entity_key() {
    let supergraph = read_supergraph("fixture/tests/context-edge-cases.supergraph.graphql");
    let transaction =
        r#"type Transaction @join__type(graph: A, key: "id") @join__type(graph: B, key: "id")"#;
    for supergraph in [
        // no key
        supergraph.replace(
            transaction,
            "type Transaction @join__type(graph: A) @join__type(graph: B)",
        ),
        // non-resolvable key
        supergraph.replace(
            transaction,
            r#"type Transaction @join__type(graph: A, key: "id", resolvable: false) @join__type(graph: B, key: "id", resolvable: false)"#,
        ),
        // key field is external
        supergraph.replace(
            "id: ID!\n    base:",
            "id: ID! @join__field(graph: A, external: true) @join__field(graph: B, external: true)\n    base:",
        ),
    ] {
        let error = build_query_plan_from_sdl(
            &supergraph,
            parse_operation("{ users { txs { amount } } }"),
            Default::default(),
            Default::default(),
        )
        .unwrap_err();

        insta::allow_duplicates! {
            insta::assert_snapshot!(error, @"No paths found for selection item: amount");
        }
    }
}
