use super::testkit::{build_query_plan_from_sdl, build_query_plan_with_defaults, read_supergraph};
use crate::query_planner::utils::parsing::parse_operation;
use std::error::Error;

#[test]
fn non_null_context_variables_disable_group_aliasing() -> Result<(), Box<dyn Error>> {
    use crate::query_planner::planner::plan_nodes::planning::PlanNode;

    let sdl = read_supergraph("fixture/tests/context-basic.supergraph.graphql");
    for (variable_type, expected_limit) in [("String", 15), ("String!", 1)] {
        let plan = build_query_plan_from_sdl(
            &sdl.replace("type: \"String\"", &format!("type: \"{variable_type}\"")),
            parse_operation("{ users { txs { amount } } }"),
            Default::default(),
            Default::default(),
        )?;
        let Some(PlanNode::Sequence(sequence)) = plan.node else {
            panic!("sequence");
        };
        let Some(PlanNode::Flatten(flatten)) = sequence.nodes.last() else {
            panic!("flatten");
        };
        let PlanNode::Fetch(fetch) = flatten.node.as_ref() else {
            panic!("fetch");
        };
        assert_eq!(
            fetch.operation.operation.max_context_groups(),
            expected_limit
        );
    }
    Ok(())
}

#[test]
fn mutation_results_supply_context_in_root_field_order() -> Result<(), Box<dyn Error>> {
    let plan = build_query_plan_with_defaults(
        "fixture/tests/context-basic.supergraph.graphql",
        parse_operation(
            r#"mutation { first: updateUser(id: "1") { txs { amount } } second: updateUser(id: "2") { txs { amount } } }"#,
        ),
    )?;
    insta::assert_snapshot!(plan.to_string(), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          mutation {
            first: updateUser(id: "1") {
              __typename
              currency
              txs {
                __typename
                id
              }
            }
          }
        },
        Flatten(path: "first.txs.@") {
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
        Fetch(service: "a") {
          mutation {
            second: updateUser(id: "2") {
              __typename
              currency
              txs {
                __typename
                id
              }
            }
          }
        },
        Flatten(path: "second.txs.@") {
          Fetch(service: "a") {
            {
              ... on Transaction {
                __typename
                id
              }
            } =>
            ($__context_1:String) {
              ... on Transaction {
                amount(code: $__context_1)
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
fn multi_type_fetch_preserves_contexts_from_different_ancestors() -> Result<(), Box<dyn Error>> {
    let plan = build_query_plan_with_defaults(
        "fixture/tests/context-outer.supergraph.graphql",
        parse_operation("{ outer { user { entries { ... on Transaction { amount } ... on Adjustment { amount } } } } }"),
    )?;
    insta::assert_snapshot!(plan.to_string(), @r#"
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
                entries {
                  __typename
                  ... on Transaction {
                    __typename
                    id
                  }
                  ... on Adjustment {
                    __typename
                    id
                  }
                }
              }
            }
          }
        },
        Flatten(path: "outer.user.entries.@") {
          Fetch(service: "a") {
            {
              ... on Adjustment {
                __typename
                id
              }
              ... on Transaction {
                __typename
                id
              }
            } =>
            ($__context_1:String,$__context_0:String) {
              ... on Adjustment {
                amount(code: $__context_1)
              }
              ... on Transaction {
                amount(code: $__context_0)
              }
            }
          },
        },
      },
    },
    "#);
    insta::assert_json_snapshot!(plan, @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Sequence",
        "nodes": [
          {
            "kind": "Fetch",
            "serviceName": "a",
            "operationKind": "query",
            "operation": "{outer{__typename code user{__typename currency entries{__typename ...on Transaction{__typename id} ...on Adjustment{__typename id}}}}}"
          },
          {
            "kind": "Flatten",
            "path": [
              {
                "Field": "outer"
              },
              {
                "Field": "user"
              },
              {
                "Field": "entries"
              },
              "@"
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "a",
              "variableUsages": [
                "__context_0",
                "__context_1"
              ],
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!, $__context_1:String, $__context_0:String){_entities(representations: $representations){...on Adjustment{amount(code: $__context_1)} ...on Transaction{amount(code: $__context_0)}}}",
              "requires": [
                {
                  "kind": "InlineFragment",
                  "typeCondition": "Adjustment",
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
                },
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
                "__context_1",
                "__context_0"
              ],
              "contextRewrites": [
                {
                  "path": [
                    "Parent",
                    "Parent",
                    {
                      "TypenameEquals": [
                        "Outer"
                      ]
                    },
                    {
                      "Key": "code"
                    }
                  ],
                  "renameKeyTo": "__context_1"
                },
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
fn context_variables_do_not_collide_with_client_variables_or_requires() -> Result<(), Box<dyn Error>>
{
    let plan = build_query_plan_with_defaults(
        "fixture/tests/context-basic.supergraph.graphql",
        parse_operation("query($__context_0: Int = 2, $__context_1: Boolean!, $__context_3: Int) { users { txs { required(multiplier: $__context_0) @include(if: $__context_1) amount(multiplier: $__context_3) } } }"),
    )?;
    insta::assert_snapshot!(plan.to_string(), @r#"
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
        Include(if: $__context_1) {
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
        },
        Flatten(path: "users.@.txs.@") {
          Fetch(service: "a") {
            {
              ... on Transaction {
                __typename
                id
                base
              }
            } =>
            ($__context_0:Int=2,$__context_1:Boolean!,$__context_3:Int,$__context_4:String,$__context_2:String) {
              ... on Transaction {
                amount(code: $__context_4, multiplier: $__context_3)
                ... on Transaction @include(if: $__context_1) {
                  required(code: $__context_2, multiplier: $__context_0)
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
fn batch_context_variables_do_not_collide_with_client_variables() -> Result<(), Box<dyn Error>> {
    let plan = build_query_plan_with_defaults(
        "fixture/tests/context-basic.supergraph.graphql",
        parse_operation("query($__context_0: Int!, $__batch_reps_0: Int!) { users { txs { amount(multiplier: $__context_0) } } secondUsers { txs { amount(multiplier: $__batch_reps_0) } } }"),
    )?;
    insta::assert_snapshot!(plan.to_string(), @r#"
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
          ($__batch_reps_0:Int!,$__context_2:String,$__context_0:Int!,$__context_1:String) {
            _e0: _entities(representations: $__batch_reps_1) {
              ... on Transaction {
                amount(code: $__context_2, multiplier: $__batch_reps_0)
              }
            }
            _e1: _entities(representations: $__batch_reps_2) {
              ... on Transaction {
                amount(code: $__context_1, multiplier: $__context_0)
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
fn context_under_conditional_ancestor_fragments() -> Result<(), Box<dyn Error>> {
    let plan = build_query_plan_with_defaults(
        "fixture/tests/context-basic.supergraph.graphql",
        parse_operation("query($skip: Boolean!, $include: Boolean!) { viewer { ... on User @include(if: $include) { txs { fromUnion @skip(if: $skip) } } ... on Other @skip(if: $skip) { txs { fromUnion @include(if: $include) } } } }"),
    )?;
    insta::assert_snapshot!(plan.to_string(), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          query ($include:Boolean!,$skip:Boolean!) {
            viewer {
              __typename
              ... on User {
                __typename
                currency @include(if: $include)
                txs @include(if: $include) {
                  ...a
                }
              }
              ... on Other {
                __typename
                currency @skip(if: $skip)
                txs @skip(if: $skip) {
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
        Parallel {
          Skip(if: $skip) {
            Include(if: $include) {
              Flatten(path: "viewer|[Other].txs.@") {
                Fetch(service: "a") {
                  {
                    ... on Transaction {
                      __typename
                      id
                    }
                  } =>
                  ($__context_1:String) {
                    ... on Transaction {
                      fromUnion(code: $__context_1)
                    }
                  }
                },
              },
            },
          },
          Include(if: $include) {
            Skip(if: $skip) {
              Flatten(path: "viewer|[User].txs.@") {
                Fetch(service: "a") {
                  {
                    ... on Transaction {
                      __typename
                      id
                    }
                  } =>
                  ($__context_0:String) {
                    ... on Transaction {
                      fromUnion(code: $__context_0)
                    }
                  }
                },
              },
            },
          },
        },
      },
    },
    "#);
    Ok(())
}

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
        "fixture/tests/context-outer.supergraph.graphql",
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
fn multi_type_fetch_does_not_require_contexts_from_mutually_exclusive_ancestors(
) -> Result<(), Box<dyn Error>> {
    let plan = build_query_plan_with_defaults(
        "fixture/tests/context-multi-type.supergraph.graphql",
        parse_operation(
            "{ viewer { ... on User { txs { amount } } ... on Other { txs { amount } } } }",
        ),
    )?;
    let plan = plan.to_string();
    assert!(!plan.contains("BatchFetch"));
    assert!(
        plan.contains("Flatten(path: \"viewer|[User].txs.@\")"),
        "{plan}"
    );
    assert!(
        plan.contains("Flatten(path: \"viewer|[Other].txs.@\")"),
        "{plan}"
    );
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

#[test]
fn context_from_the_implementation_of_an_interface_ancestor() -> Result<(), Box<dyn Error>> {
    let plan = build_query_plan_with_defaults(
        "fixture/tests/context-interface-implementation.supergraph.graphql",
        parse_operation("{ accounts { wallet { discount } } }"),
    )?;

    insta::assert_snapshot!(plan.to_string(), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          {
            accounts {
              __typename
              ... on Member {
                plan
              }
              ... on Guest {
                plan
              }
              wallet {
                __typename
                id
              }
            }
          }
        },
        Flatten(path: "accounts.@.wallet") {
          Fetch(service: "a") {
            {
              ... on Wallet {
                __typename
                id
              }
            } =>
            ($__context_0:String) {
              ... on Wallet {
                discount(plan: $__context_0)
              }
            }
          },
        },
      },
    },
    "#);
    let plan = serde_json::to_value(plan)?;
    assert_eq!(
        plan["node"]["nodes"][1]["node"]["contextRewrites"],
        serde_json::json!([
            {
                "path": ["Parent", {"TypenameEquals": ["Member"]}, {"Key": "plan"}],
                "renameKeyTo": "__context_0"
            },
            {
                "path": ["Parent", {"TypenameEquals": ["Guest"]}, {"Key": "plan"}],
                "renameKeyTo": "__context_0"
            }
        ])
    );
    Ok(())
}

// The tests below plan against supergraphs composed from real subgraphs, one for each shape
// that composition (federation-composition#335) accepts.

fn plan_composed(fixture: &str, query: &str) -> Result<String, Box<dyn Error>> {
    Ok(build_query_plan_with_defaults(
        &format!("fixture/tests/{fixture}.supergraph.graphql"),
        parse_operation(query),
    )?
    .to_string())
}

#[test]
fn requires_selection_that_sets_its_own_context() -> Result<(), Box<dyn Error>> {
    // `b` requires `offer { value }`, and `value` reads the context of `plan`, which is the
    // parent of the requiring field
    let plan = plan_composed("context-with-directives", "{ plan { discount } }")?;
    insta::assert_snapshot!(plan, @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          {
            plan {
              __typename
              id
              tier
              offer {
                __typename
                id
              }
            }
          }
        },
        Flatten(path: "plan.offer") {
          Fetch(service: "a") {
            {
              ... on Offer {
                __typename
                id
              }
            } =>
            ($__context_0:String) {
              ... on Offer {
                value(tier: $__context_0)
              }
            }
          },
        },
        Flatten(path: "plan") {
          Fetch(service: "b") {
            {
              ... on Plan {
                __typename
                offer {
                  value
                }
                id
              }
            } =>
            {
              ... on Plan {
                discount
              }
            }
          },
        },
      },
    },
    "#);
    // the client selects the same contextual field the requirement selects
    let plan = plan_composed(
        "context-with-directives",
        "{ plan { discount offer { value } } }",
    )?;
    insta::assert_snapshot!(plan, @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          {
            plan {
              __typename
              id
              tier
              offer {
                __typename
                id
              }
            }
          }
        },
        BatchFetch(service: "a") {
          {
            _e0 {
              paths: [
                "plan.offer"
              ]
              {
                ... on Offer {
                  __typename
                  id
                }
              }
            }
            _e1 {
              paths: [
                "plan.offer"
              ]
              {
                ... on Offer {
                  __typename
                  id
                }
              }
            }
          }
          ($__context_0:String,$__context_1:String) {
            _e0: _entities(representations: $__batch_reps_0) {
              ... on Offer {
                value(tier: $__context_0)
              }
            }
            _e1: _entities(representations: $__batch_reps_1) {
              ... on Offer {
                value(tier: $__context_1)
              }
            }
          }
        },
        Flatten(path: "plan") {
          Fetch(service: "b") {
            {
              ... on Plan {
                __typename
                offer {
                  value
                }
                id
              }
            } =>
            {
              ... on Plan {
                discount
              }
            }
          },
        },
      },
    },
    "#);
    Ok(())
}
