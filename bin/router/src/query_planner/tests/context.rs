use super::testkit::{
    build_query_plan_from_sdl, build_query_plan_with_defaults, read_supergraph,
    read_supergraph_state,
};
use crate::query_planner::state::supergraph_state::SupergraphState;
use crate::query_planner::utils::parsing::parse_operation;
use crate::query_planner::utils::parsing::parse_schema;
use std::error::Error;

#[test]
fn non_null_context_variables_disable_group_aliasing() -> Result<(), Box<dyn Error>> {
    use crate::query_planner::planner::plan_nodes::planning::PlanNode;

    // a non-null contextual argument only composes with a `__typename` selection
    for (fixture, query, expected_limit) in [
        ("context-basic", "{ users { txs { amount } } }", usize::MAX),
        (
            "context-non-null",
            "{ viewer { ... on User { txs { amount } } } }",
            1,
        ),
    ] {
        let plan = build_query_plan_with_defaults(
            &format!("fixture/tests/{fixture}.supergraph.graphql"),
            parse_operation(query),
        )?;
        let Some(PlanNode::Sequence(sequence)) = plan.into_node() else {
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

/// The planner compiles a template for every contextual Flatten and BatchFetch, it survives
/// turning the plan into an executable one and cloning it, and what it writes parses.
#[test]
fn contextual_fetches_carry_a_compiled_template() -> Result<(), Box<dyn Error>> {
    use crate::query_planner::{
        ast::operation::SubgraphFetchOperation,
        planner::plan_nodes::{FetchContext, PlanNode},
    };

    fn contextual<'a>(
        node: &'a PlanNode,
        found: &mut Vec<(&'a SubgraphFetchOperation, Vec<String>)>,
    ) {
        match node {
            PlanNode::Fetch(fetch) if !FetchContext::variables(&fetch.context).is_empty() => {
                let variables = FetchContext::group_variables(&fetch.context)
                    .into_iter()
                    .chain(["representations"])
                    .map(str::to_string)
                    .collect();
                found.push((&fetch.operation, variables));
            }
            PlanNode::BatchFetch(batch)
                if batch
                    .entity_batch
                    .aliases
                    .iter()
                    .any(|alias| !FetchContext::variables(&alias.context).is_empty()) =>
            {
                let variables = batch
                    .entity_batch
                    .aliases
                    .iter()
                    .flat_map(|alias| {
                        FetchContext::group_variables(&alias.context)
                            .into_iter()
                            .chain([alias.representations_variable_name.as_str()])
                            .map(str::to_string)
                    })
                    .collect();
                found.push((&batch.operation, variables));
            }
            PlanNode::Sequence(node) => node.nodes.iter().for_each(|node| contextual(node, found)),
            PlanNode::Parallel(node) => node.nodes.iter().for_each(|node| contextual(node, found)),
            PlanNode::Flatten(node) => contextual(&node.node, found),
            _ => {}
        }
    }

    for (fixture, query, batched) in [
        ("context-basic", "{ users { txs { amount } } }", false),
        (
            "context-shared-duck",
            "{ foo { u { field } } bar { u { field } } }",
            true,
        ),
    ] {
        let plan = build_query_plan_with_defaults(
            &format!("fixture/tests/{fixture}.supergraph.graphql"),
            parse_operation(query),
        )?
        .into_executable(&read_supergraph_state(&format!(
            "fixture/tests/{fixture}.supergraph.graphql"
        )));
        // plans are cloned out of the cache
        let plan = plan.clone();
        let mut found = Vec::new();
        contextual(plan.node().unwrap(), &mut found);
        assert!(!found.is_empty(), "{fixture}");
        assert_eq!(
            found
                .iter()
                .any(|(operation, _)| operation.document_str.contains("__batch_reps")),
            batched,
            "{fixture}"
        );
        for (operation, variables) in found {
            let template = operation
                .context_alias_template()
                .unwrap_or_else(|| panic!("no template for {}", operation.document_str));
            let written = template.operation(2);
            graphql_tools::parser::parse_query::<String>(&written.document_str)?;
            for variable in variables {
                for group in 0..2 {
                    assert!(
                        written
                            .document_str
                            .contains(&format!("${variable}_{group}:")),
                        "{variable} in group {group} of {}",
                        written.document_str
                    );
                }
            }
        }
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
    fetches
      fetch               after
      [2] a               -
      [3] a               [2]
      [4] a               [3]
      [5] a               [4]

    [2] a
      out
        { first: updateUser(id: "1") { __typename currency txs { __typename id } } }
      mutation

    [3] a   after [2]
      merge
        first.txs.@
      in
        ... on Transaction { __typename id }
      out
        ... on Transaction { amount(code: $__context_0) }
      vars
        $__context_0
      context
        $__context_0 = ..|[User]/currency

    [4] a   after [3]
      out
        { second: updateUser(id: "2") { __typename currency txs { __typename id } } }
      mutation

    [5] a   after [4]
      merge
        second.txs.@
      in
        ... on Transaction { __typename id }
      out
        ... on Transaction { amount(code: $__context_1) }
      vars
        $__context_1
      context
        $__context_1 = ..|[User]/currency
    "#);
    Ok(())
}

#[test]
fn multi_type_fetch_preserves_contexts_from_different_ancestors() -> Result<(), Box<dyn Error>> {
    let plan = build_query_plan_with_defaults(
        "fixture/tests/context-outer.supergraph.graphql",
        parse_operation("{ outer { user { entries { ... on Transaction { amount } ... on Adjustment { amount } } } } }"),
    )?;
    insta::assert_snapshot!(plan.to_string(), @r"
    fetches
      fetch               after
      [2] a               -
      [4] a               [2]

    [2] a
      out
        {
          outer {
            __typename
            code
            user {
              __typename
              currency
              entries {
                __typename
                ... on Transaction { __typename id }
                ... on Adjustment { __typename id }
              }
            }
          }
        }

    [4] a   after [2]
      merge
        outer.user.entries.@
      in
        ... on Adjustment { __typename id } ... on Transaction { __typename id }
      out
        ... on Adjustment { amount(code: $__context_1) }
        ... on Transaction { amount(code: $__context_0) }
      vars
        $__context_0, $__context_1
      context
        $__context_1 = ../..|[Outer]/code
        $__context_0 = ..|[User]/currency
    ");
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
    insta::assert_snapshot!(plan.to_string(), @r"
    variables
      $__context_0: Int = 2, $__context_1, $__context_3: Int

    fetches
      fetch               after
      [2] a               -
      [5] b               [2]
      [6] a               [2]
      [3] a               [5]

    [2] a
      out
        { users { __typename currency txs { __typename id } } }

    [5] b   after [2]
      if
        $__context_1
      merge
        users.@.txs.@
      in
        ... on Transaction { __typename id }
      out
        ... on Transaction { base }

    [6] a   after [2]
      merge
        users.@.txs.@
      in
        ... on Transaction { __typename id }
      out
        ... on Transaction { amount(code: $__context_4, multiplier: $__context_3) }
      vars
        $__context_3, $__context_4
      context
        $__context_4 = ..|[User]/currency

    [3] a   after [5]
      if
        $__context_1
      merge
        users.@.txs.@
      in
        ... on Transaction { __typename base id }
      out
        ... on Transaction { required(code: $__context_2, multiplier: $__context_0) }
      vars
        $__context_0, $__context_2
      context
        $__context_2 = ..|[User]/currency
    ");
    Ok(())
}

#[test]
fn batch_context_variables_do_not_collide_with_client_variables() -> Result<(), Box<dyn Error>> {
    let plan = build_query_plan_with_defaults(
        "fixture/tests/context-basic.supergraph.graphql",
        parse_operation("query($__context_0: Int!, $__batch_reps_0: Int!) { users { txs { amount(multiplier: $__context_0) } } secondUsers { txs { amount(multiplier: $__batch_reps_0) } } }"),
    )?;
    insta::assert_snapshot!(plan.to_string(), @r"
    variables
      $__batch_reps_0: Int!, $__context_0: Int!

    fetches
      fetch               after
      [2] a               -
      [4] a               [2]

    [2] a
      out
        { users { ...a } secondUsers { ...a } }
        fragment a on User { __typename currency txs { __typename id } }

    [4] a   after [2]
      alias _e0
        merge
          secondUsers.@.txs.@
        in
          ... on Transaction { __typename id }
        out
          ... on Transaction { amount(code: $__context_2, multiplier: $__batch_reps_0) }
        context
          $__context_2 = ..|[User]/currency
      alias _e1
        merge
          users.@.txs.@
        in
          ... on Transaction { __typename id }
        out
          ... on Transaction { amount(code: $__context_1, multiplier: $__context_0) }
        context
          $__context_1 = ..|[User]/currency
      vars
        $__batch_reps_0, $__context_0, $__context_1, $__context_2
    ");
    Ok(())
}

#[test]
fn context_under_conditional_ancestor_fragments() -> Result<(), Box<dyn Error>> {
    let plan = build_query_plan_with_defaults(
        "fixture/tests/context-basic.supergraph.graphql",
        parse_operation("query($skip: Boolean!, $include: Boolean!) { viewer { ... on User @include(if: $include) { txs { fromUnion @skip(if: $skip) } } ... on Other @skip(if: $skip) { txs { fromUnion @include(if: $include) } } } }"),
    )?;
    insta::assert_snapshot!(plan.to_string(), @r"
    variables
      $include: Boolean!, $skip: Boolean!

    fetches
      fetch               after
      [2] a               -
      [4] a               [2]

    [2] a
      out
        {
          viewer {
            __typename
            ... on User {
              __typename
              currency @include(if: $include)
              txs @include(if: $include) { ...a }
            }
            ... on Other { __typename currency @skip(if: $skip) txs @skip(if: $skip) { ...a } }
          }
        }
        fragment a on Transaction { __typename id }
      vars
        $include, $skip

    [4] a   after [2]
      alias _e0
        if
          $include && !$skip
        merge
          viewer|[Other].txs.@
        in
          ... on Transaction { __typename id }
        out
          ... on Transaction { fromUnion(code: $__context_1) }
        context
          $__context_1 = ..|[User]/currency
          $__context_1 = ..|[Other]/currency
      alias _e1
        if
          $include && !$skip
        merge
          viewer|[User].txs.@
        in
          ... on Transaction { __typename id }
        out
          ... on Transaction { fromUnion(code: $__context_0) }
        context
          $__context_0 = ..|[User]/currency
          $__context_0 = ..|[Other]/currency
      vars
        $__context_0, $__context_1
    ");
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
    let plan = sonic_rs::to_value(
        &plan.into_executable(&SupergraphState::new(&parse_schema(&supergraph))),
    )?;
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

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] a               -
      [3] a               [2]

    [2] a
      out
        { users { __typename currency txs { __typename id } } }

    [3] a   after [2]
      merge
        users.@.txs.@
      in
        ... on Transaction { __typename id }
      out
        ... on Transaction { amount(code: $__context_0) }
      vars
        $__context_0
      context
        $__context_0 = ..|[User]/currency
    ");
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

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] a               -
      [3] a               [2]

    [2] a
      out
        { users { child { __typename currency txs { __typename id } } } }

    [3] a   after [2]
      merge
        users.@.child.txs.@
      in
        ... on Transaction { __typename id }
      out
        ... on Transaction { amount(code: $__context_0) }
      vars
        $__context_0
      context
        $__context_0 = ..|[User]/currency
    ");

    Ok(())
}

#[test]
fn ancestor_through_non_context_wrapper() -> Result<(), Box<dyn Error>> {
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/context-basic.supergraph.graphql",
        parse_operation(r#"{ users { wrapper { txs { amount } } } }"#),
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] a               -
      [3] a               [2]

    [2] a
      out
        { users { __typename currency wrapper { txs { __typename id } } } }

    [3] a   after [2]
      merge
        users.@.wrapper.txs.@
      in
        ... on Transaction { __typename id }
      out
        ... on Transaction { amount(code: $__context_0) }
      vars
        $__context_0
      context
        $__context_0 = ../..|[User]/currency
    ");

    Ok(())
}

#[test]
fn interface_context() -> Result<(), Box<dyn Error>> {
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/context-basic.supergraph.graphql",
        parse_operation(r#"{ account { txs { fromInterface } } }"#),
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] a               -
      [3] a               [2]

    [2] a
      out
        { account { __typename currency txs { __typename id } } }

    [3] a   after [2]
      merge
        account.txs.@
      in
        ... on Transaction { __typename id }
      out
        ... on Transaction { fromInterface(code: $__context_0) }
      vars
        $__context_0
      context
        $__context_0 = ..|[Account]/currency
    ");

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
            "operation": "{account{__typename currency txs{__typename id}}}"
          },
          {
            "kind": "Flatten",
            "path": [
              {
                "Field": "account"
              },
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
              "operation": "query($representations:[_Any!]!, $__context_0:String){_entities(representations: $representations){...on Transaction{fromInterface(code: $__context_0)}}}",
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
                        "Account"
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
fn remote_context_producer_precedes_consumer() -> Result<(), Box<dyn Error>> {
    let plan = build_query_plan_with_defaults(
        "fixture/tests/context-remote-producer.supergraph.graphql",
        parse_operation("{ t { u { id field } } }"),
    )?;
    insta::assert_snapshot!(plan.to_string(), @r"
    fetches
      fetch               after
      [2] a               -
      [3] b               [2]
      [4] a               [3]

    [2] a
      out
        { t { __typename id u { __typename id } } }

    [3] b   after [2]
      merge
        t
      in
        ... on T { __typename id }
      out
        ... on T { prop }

    [4] a   after [3]
      merge
        t.u
      in
        ... on U { __typename id }
      out
        ... on U { field(a: $__context_0) }
      vars
        $__context_0
      context
        $__context_0 = ..|[T]/prop
    ");
    insta::assert_json_snapshot!(plan.into_executable(&read_supergraph_state("fixture/tests/context-remote-producer.supergraph.graphql")), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Sequence",
        "nodes": [
          {
            "kind": "Fetch",
            "serviceName": "a",
            "operationKind": "query",
            "operation": "{t{__typename id u{__typename id}}}"
          },
          {
            "kind": "Flatten",
            "path": [
              {
                "Field": "t"
              }
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "b",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on T{prop}}}",
              "requires": [
                {
                  "kind": "InlineFragment",
                  "typeCondition": "T",
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
              {
                "Field": "t"
              },
              {
                "Field": "u"
              }
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "a",
              "variableUsages": [
                "__context_0"
              ],
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!, $__context_0:String){_entities(representations: $representations){...on U{field(a: $__context_0)}}}",
              "requires": [
                {
                  "kind": "InlineFragment",
                  "typeCondition": "U",
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
                        "T"
                      ]
                    },
                    {
                      "Key": "prop"
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
fn context_selection_crosses_subgraphs_multiple_times() -> Result<(), Box<dyn Error>> {
    let plan = build_query_plan_with_defaults(
        "fixture/tests/context-multi-hop.supergraph.graphql",
        parse_operation("{ t { u { field } } }"),
    )?;
    insta::assert_snapshot!(plan.to_string(), @r"
    fetches
      fetch               after
      [2] a               -
      [3] b               [2]
      [4] a               [3]
      [5] a               [4]

    [2] a
      out
        { t { __typename a { __typename id } u { __typename id } } }

    [3] b   after [2]
      merge
        t.a
      in
        ... on A { __typename id }
      out
        ... on A { b { __typename id } }

    [4] a   after [3]
      merge
        t.a.b
      in
        ... on B { __typename id }
      out
        ... on B { c { prop } }

    [5] a   after [4]
      merge
        t.u
      in
        ... on U { __typename id }
      out
        ... on U { field(a: $__context_0) }
      vars
        $__context_0
      context
        $__context_0 = ..|[T]/a/b/c/prop
    ");
    insta::assert_json_snapshot!(plan.into_executable(&read_supergraph_state("fixture/tests/context-multi-hop.supergraph.graphql")), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Sequence",
        "nodes": [
          {
            "kind": "Fetch",
            "serviceName": "a",
            "operationKind": "query",
            "operation": "{t{__typename a{__typename id} u{__typename id}}}"
          },
          {
            "kind": "Flatten",
            "path": [
              {
                "Field": "t"
              },
              {
                "Field": "a"
              }
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "b",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on A{b{__typename id}}}}",
              "requires": [
                {
                  "kind": "InlineFragment",
                  "typeCondition": "A",
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
              {
                "Field": "t"
              },
              {
                "Field": "a"
              },
              {
                "Field": "b"
              }
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "a",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on B{c{prop}}}}",
              "requires": [
                {
                  "kind": "InlineFragment",
                  "typeCondition": "B",
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
              {
                "Field": "t"
              },
              {
                "Field": "u"
              }
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "a",
              "variableUsages": [
                "__context_0"
              ],
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!, $__context_0:String){_entities(representations: $representations){...on U{field(a: $__context_0)}}}",
              "requires": [
                {
                  "kind": "InlineFragment",
                  "typeCondition": "U",
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
                        "T"
                      ]
                    },
                    {
                      "Key": "a"
                    },
                    {
                      "Key": "b"
                    },
                    {
                      "Key": "c"
                    },
                    {
                      "Key": "prop"
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
fn query_root_context_selects_a_sibling_field() -> Result<(), Box<dyn Error>> {
    let plan = build_query_plan_with_defaults(
        "fixture/tests/context-query-root.supergraph.graphql",
        parse_operation("{ product { price } }"),
    )?;
    insta::assert_snapshot!(plan.to_string(), @r"
    fetches
      fetch               after
      [2] a               -
      [3] a               [2]

    [2] a
      out
        { __typename me { locale } product { __typename id } }

    [3] a   after [2]
      merge
        product
      in
        ... on Product { __typename id }
      out
        ... on Product { price(locale: $__context_0) }
      vars
        $__context_0
      context
        $__context_0 = ..|[Query]/me/locale
    ");
    insta::assert_json_snapshot!(plan.into_executable(&read_supergraph_state("fixture/tests/context-query-root.supergraph.graphql")), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Sequence",
        "nodes": [
          {
            "kind": "Fetch",
            "serviceName": "a",
            "operationKind": "query",
            "operation": "{__typename me{locale} product{__typename id}}"
          },
          {
            "kind": "Flatten",
            "path": [
              {
                "Field": "product"
              }
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "a",
              "variableUsages": [
                "__context_0"
              ],
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!, $__context_0:String){_entities(representations: $representations){...on Product{price(locale: $__context_0)}}}",
              "requires": [
                {
                  "kind": "InlineFragment",
                  "typeCondition": "Product",
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
                        "Query"
                      ]
                    },
                    {
                      "Key": "me"
                    },
                    {
                      "Key": "locale"
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
fn context_is_resolved_before_a_key_transition() -> Result<(), Box<dyn Error>> {
    let plan = build_query_plan_with_defaults(
        "fixture/tests/context-before-key-transition.supergraph.graphql",
        parse_operation("{ customer { child { id prop } } }"),
    )?;
    insta::assert_snapshot!(plan.to_string(), @r"
    fetches
      fetch               after
      [2] a               -
      [4] b               [2]

    [2] a
      out
        { customer { __typename identifiers { legacyUserId } child { __typename id } } }

    [4] b   after [2]
      merge
        customer.child
      in
        ... on Child { __typename id }
      out
        ... on Child { prop(legacyUserId: $__context_0) }
      vars
        $__context_0
      context
        $__context_0 = ..|[Customer]/identifiers/legacyUserId
    ");
    insta::assert_json_snapshot!(plan.into_executable(&read_supergraph_state("fixture/tests/context-before-key-transition.supergraph.graphql")), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Sequence",
        "nodes": [
          {
            "kind": "Fetch",
            "serviceName": "a",
            "operationKind": "query",
            "operation": "{customer{__typename identifiers{legacyUserId} child{__typename id}}}"
          },
          {
            "kind": "Flatten",
            "path": [
              {
                "Field": "customer"
              },
              {
                "Field": "child"
              }
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "b",
              "variableUsages": [
                "__context_0"
              ],
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!, $__context_0:ID){_entities(representations: $representations){...on Child{prop(legacyUserId: $__context_0)}}}",
              "requires": [
                {
                  "kind": "InlineFragment",
                  "typeCondition": "Child",
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
                        "Customer"
                      ]
                    },
                    {
                      "Key": "identifiers"
                    },
                    {
                      "Key": "legacyUserId"
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
fn context_value_reuses_an_existing_fetch() -> Result<(), Box<dyn Error>> {
    let plan = build_query_plan_with_defaults(
        "fixture/tests/context-existing-fetch.supergraph.graphql",
        parse_operation("{ t { u { id field } } }"),
    )?;
    insta::assert_snapshot!(plan.to_string(), @r"
    fetches
      fetch               after
      [2] a               -
      [4] b               [2]

    [2] a
      out
        { t { __typename prop u { __typename id } } }

    [4] b   after [2]
      merge
        t.u
      in
        ... on U { __typename id }
      out
        ... on U { field(a: $__context_0) }
      vars
        $__context_0
      context
        $__context_0 = ..|[T]/prop
    ");
    insta::assert_json_snapshot!(plan.into_executable(&read_supergraph_state("fixture/tests/context-existing-fetch.supergraph.graphql")), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Sequence",
        "nodes": [
          {
            "kind": "Fetch",
            "serviceName": "a",
            "operationKind": "query",
            "operation": "{t{__typename prop u{__typename id}}}"
          },
          {
            "kind": "Flatten",
            "path": [
              {
                "Field": "t"
              },
              {
                "Field": "u"
              }
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "b",
              "variableUsages": [
                "__context_0"
              ],
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!, $__context_0:String){_entities(representations: $representations){...on U{field(a: $__context_0)}}}",
              "requires": [
                {
                  "kind": "InlineFragment",
                  "typeCondition": "U",
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
                        "T"
                      ]
                    },
                    {
                      "Key": "prop"
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
fn list_context_argument_preserves_its_type() -> Result<(), Box<dyn Error>> {
    let plan = build_query_plan_with_defaults(
        "fixture/tests/context-list-argument.supergraph.graphql",
        parse_operation("{ t { u { field } } }"),
    )?;
    insta::assert_snapshot!(plan.to_string(), @r"
    fetches
      fetch               after
      [2] a               -
      [3] a               [2]

    [2] a
      out
        { t { __typename prop u { __typename id } } }

    [3] a   after [2]
      merge
        t.u
      in
        ... on U { __typename id }
      out
        ... on U { field(a: $__context_0) }
      vars
        $__context_0
      context
        $__context_0 = ..|[T]/prop
    ");
    insta::assert_json_snapshot!(plan.into_executable(&read_supergraph_state("fixture/tests/context-list-argument.supergraph.graphql")), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Sequence",
        "nodes": [
          {
            "kind": "Fetch",
            "serviceName": "a",
            "operationKind": "query",
            "operation": "{t{__typename prop u{__typename id}}}"
          },
          {
            "kind": "Flatten",
            "path": [
              {
                "Field": "t"
              },
              {
                "Field": "u"
              }
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "a",
              "variableUsages": [
                "__context_0"
              ],
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!, $__context_0:[String]){_entities(representations: $representations){...on U{field(a: $__context_0)}}}",
              "requires": [
                {
                  "kind": "InlineFragment",
                  "typeCondition": "U",
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
                        "T"
                      ]
                    },
                    {
                      "Key": "prop"
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
fn multiple_context_arguments_merge_into_an_ancestor_fetch() -> Result<(), Box<dyn Error>> {
    let plan = build_query_plan_with_defaults(
        "fixture/tests/context-merged-arguments.supergraph.graphql",
        parse_operation("{ customer { accounts { foo { id } } } }"),
    )?;
    insta::assert_snapshot!(plan.to_string(), @r"
    fetches
      fetch               after
      [2] b               -
      [3] c               [2]

    [2] b
      out
        { customer { __typename id identifiers { id5 } mid } }

    [3] c   after [2]
      merge
        customer
      in
        ... on Customer { __typename id }
      out
        ... on Customer {
          __typename
          id
          accounts { __typename id foo(ctx_id5: $__context_0, ctx_mid: $__context_1) { id } }
        }
      vars
        $__context_0, $__context_1
      context
        $__context_0 = [Customer]/identifiers/id5
        $__context_1 = [Customer]/mid
    ");
    insta::assert_json_snapshot!(plan.into_executable(&read_supergraph_state("fixture/tests/context-merged-arguments.supergraph.graphql")), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Sequence",
        "nodes": [
          {
            "kind": "Fetch",
            "serviceName": "b",
            "operationKind": "query",
            "operation": "{customer{__typename id identifiers{id5} mid}}"
          },
          {
            "kind": "Flatten",
            "path": [
              {
                "Field": "customer"
              }
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "c",
              "variableUsages": [
                "__context_0",
                "__context_1"
              ],
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!, $__context_0:ID, $__context_1:ID){_entities(representations: $representations){...on Customer{__typename id accounts{__typename id foo(ctx_id5: $__context_0, ctx_mid: $__context_1){id}}}}}",
              "requires": [
                {
                  "kind": "InlineFragment",
                  "typeCondition": "Customer",
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
                "__context_0",
                "__context_1"
              ],
              "contextRewrites": [
                {
                  "path": [
                    {
                      "TypenameEquals": [
                        "Customer"
                      ]
                    },
                    {
                      "Key": "identifiers"
                    },
                    {
                      "Key": "id5"
                    }
                  ],
                  "renameKeyTo": "__context_0"
                },
                {
                  "path": [
                    {
                      "TypenameEquals": [
                        "Customer"
                      ]
                    },
                    {
                      "Key": "mid"
                    }
                  ],
                  "renameKeyTo": "__context_1"
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
fn unrelated_objects_share_a_context_name() -> Result<(), Box<dyn Error>> {
    let plan = build_query_plan_with_defaults(
        "fixture/tests/context-shared-duck.supergraph.graphql",
        parse_operation("{ foo { u { field } } bar { u { field } } }"),
    )?;
    insta::assert_snapshot!(plan.to_string(), @r"
    fetches
      fetch               after
      [2] a               -
      [4] a               [2]

    [2] a
      out
        { foo { __typename prop u { ...a } } bar { __typename prop u { ...a } } }
        fragment a on U { __typename id }

    [4] a   after [2]
      alias _e0
        merge
          bar.u
        in
          ... on U { __typename id }
        out
          ... on U { field(a: $__context_1) }
        context
          $__context_1 = ..|[Bar]/prop
      alias _e1
        merge
          foo.u
        in
          ... on U { __typename id }
        out
          ... on U { field(a: $__context_0) }
        context
          $__context_0 = ..|[Foo]/prop
      vars
        $__context_0, $__context_1
    ");
    insta::assert_json_snapshot!(plan.into_executable(&read_supergraph_state("fixture/tests/context-shared-duck.supergraph.graphql")), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Sequence",
        "nodes": [
          {
            "kind": "Fetch",
            "serviceName": "a",
            "operationKind": "query",
            "operation": "{foo{__typename prop u{...a}} bar{__typename prop u{...a}}}\n\nfragment a on U {__typename id}\n"
          },
          {
            "kind": "BatchFetch",
            "serviceName": "a",
            "variableUsages": [
              "__context_0",
              "__context_1"
            ],
            "operationKind": "query",
            "operation": "query($__batch_reps_0:[_Any!]!, $__batch_reps_1:[_Any!]!, $__context_1:String, $__context_0:String){_e0: _entities(representations: $__batch_reps_0){...on U{field(a: $__context_1)}} _e1: _entities(representations: $__batch_reps_1){...on U{field(a: $__context_0)}}}",
            "entityBatch": {
              "aliases": [
                {
                  "alias": "_e0",
                  "representationsVariableName": "__batch_reps_0",
                  "paths": [
                    [
                      {
                        "Field": "bar"
                      },
                      {
                        "Field": "u"
                      }
                    ]
                  ],
                  "requires": [
                    {
                      "kind": "InlineFragment",
                      "typeCondition": "U",
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
                    "__context_1"
                  ],
                  "contextRewrites": [
                    {
                      "path": [
                        "Parent",
                        {
                          "TypenameEquals": [
                            "Bar"
                          ]
                        },
                        {
                          "Key": "prop"
                        }
                      ],
                      "renameKeyTo": "__context_1"
                    }
                  ]
                },
                {
                  "alias": "_e1",
                  "representationsVariableName": "__batch_reps_1",
                  "paths": [
                    [
                      {
                        "Field": "foo"
                      },
                      {
                        "Field": "u"
                      }
                    ]
                  ],
                  "requires": [
                    {
                      "kind": "InlineFragment",
                      "typeCondition": "U",
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
                            "Foo"
                          ]
                        },
                        {
                          "Key": "prop"
                        }
                      ],
                      "renameKeyTo": "__context_0"
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
fn shared_context_name_selects_fields_by_type() -> Result<(), Box<dyn Error>> {
    let plan = build_query_plan_with_defaults(
        "fixture/tests/context-shared-conditions.supergraph.graphql",
        parse_operation("{ foo { u { field } } bar { u { field } } }"),
    )?;
    insta::assert_snapshot!(plan.to_string(), @r"
    fetches
      fetch               after
      [2] a               -
      [4] a               [2]

    [2] a
      out
        { foo { __typename prop u { ...a } } bar { __typename prop2 u { ...a } } }
        fragment a on U { __typename id }

    [4] a   after [2]
      alias _e0
        merge
          bar.u
        in
          ... on U { __typename id }
        out
          ... on U { field(a: $__context_1) }
        context
          $__context_1 = ..|[Bar]/prop2
      alias _e1
        merge
          foo.u
        in
          ... on U { __typename id }
        out
          ... on U { field(a: $__context_0) }
        context
          $__context_0 = ..|[Foo]/prop
      vars
        $__context_0, $__context_1
    ");
    insta::assert_json_snapshot!(plan.into_executable(&read_supergraph_state("fixture/tests/context-shared-conditions.supergraph.graphql")), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Sequence",
        "nodes": [
          {
            "kind": "Fetch",
            "serviceName": "a",
            "operationKind": "query",
            "operation": "{foo{__typename prop u{...a}} bar{__typename prop2 u{...a}}}\n\nfragment a on U {__typename id}\n"
          },
          {
            "kind": "BatchFetch",
            "serviceName": "a",
            "variableUsages": [
              "__context_0",
              "__context_1"
            ],
            "operationKind": "query",
            "operation": "query($__batch_reps_0:[_Any!]!, $__batch_reps_1:[_Any!]!, $__context_1:String, $__context_0:String){_e0: _entities(representations: $__batch_reps_0){...on U{field(a: $__context_1)}} _e1: _entities(representations: $__batch_reps_1){...on U{field(a: $__context_0)}}}",
            "entityBatch": {
              "aliases": [
                {
                  "alias": "_e0",
                  "representationsVariableName": "__batch_reps_0",
                  "paths": [
                    [
                      {
                        "Field": "bar"
                      },
                      {
                        "Field": "u"
                      }
                    ]
                  ],
                  "requires": [
                    {
                      "kind": "InlineFragment",
                      "typeCondition": "U",
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
                    "__context_1"
                  ],
                  "contextRewrites": [
                    {
                      "path": [
                        "Parent",
                        {
                          "TypenameEquals": [
                            "Bar"
                          ]
                        },
                        {
                          "Key": "prop2"
                        }
                      ],
                      "renameKeyTo": "__context_1"
                    }
                  ]
                },
                {
                  "alias": "_e1",
                  "representationsVariableName": "__batch_reps_1",
                  "paths": [
                    [
                      {
                        "Field": "foo"
                      },
                      {
                        "Field": "u"
                      }
                    ]
                  ],
                  "requires": [
                    {
                      "kind": "InlineFragment",
                      "typeCondition": "U",
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
                            "Foo"
                          ]
                        },
                        {
                          "Key": "prop"
                        }
                      ],
                      "renameKeyTo": "__context_0"
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
fn union_context_with_type_conditions() -> Result<(), Box<dyn Error>> {
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/context-basic.supergraph.graphql",
        parse_operation(
            r#"{ viewer { ... on User { txs { fromUnion } } ... on Other { txs { fromUnion } } } }"#,
        ),
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] a               -
      [4] a               [2]

    [2] a
      out
        {
          viewer {
            __typename
            ... on User { __typename currency txs { ...a } }
            ... on Other { __typename currency txs { ...a } }
          }
        }
        fragment a on Transaction { __typename id }

    [4] a   after [2]
      alias _e0
        merge
          viewer|[Other].txs.@
        in
          ... on Transaction { __typename id }
        out
          ... on Transaction { fromUnion(code: $__context_1) }
        context
          $__context_1 = ..|[User]/currency
          $__context_1 = ..|[Other]/currency
      alias _e1
        merge
          viewer|[User].txs.@
        in
          ... on Transaction { __typename id }
        out
          ... on Transaction { fromUnion(code: $__context_0) }
        context
          $__context_0 = ..|[User]/currency
          $__context_0 = ..|[Other]/currency
      vars
        $__context_0, $__context_1
    ");
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
                  "contextVariables": [
                    "__context_1"
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

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] a               -
      [3] a               [2]

    [2] a
      out
        { outer { __typename code user { __typename currency txs { __typename id } } } }

    [3] a   after [2]
      merge
        outer.user.txs.@
      in
        ... on Transaction { __typename id }
      out
        ... on Transaction { multiple(code: $__context_0, outer: $__context_1) }
      vars
        $__context_0, $__context_1
      context
        $__context_0 = ..|[User]/currency
        $__context_1 = ../..|[Outer]/code
    ");

    Ok(())
}

#[test]
fn requires_and_context_on_one_field() -> Result<(), Box<dyn Error>> {
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/context-basic.supergraph.graphql",
        parse_operation(r#"{ users { txs { required } } }"#),
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] a               -
      [5] b               [2]
      [3] a               [5]

    [2] a
      out
        { users { __typename currency txs { __typename id } } }

    [5] b   after [2]
      merge
        users.@.txs.@
      in
        ... on Transaction { __typename id }
      out
        ... on Transaction { base }

    [3] a   after [5]
      merge
        users.@.txs.@
      in
        ... on Transaction { __typename base id }
      out
        ... on Transaction { required(code: $__context_0) }
      vars
        $__context_0
      context
        $__context_0 = ..|[User]/currency
    ");

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

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    variables
      $include

    fetches
      fetch               after
      [2] a               -
      [3] a               [2]

    [2] a
      out
        { users { __typename currency txs { __typename id } } }

    [3] a   after [2]
      if
        $include
      merge
        users.@.txs.@
      in
        ... on Transaction { __typename id }
      out
        ... on Transaction { amount(code: $__context_0) }
      vars
        $__context_0
      context
        $__context_0 = ..|[User]/currency
    ");

    Ok(())
}

#[test]
fn client_alias_colliding_with_context_key() -> Result<(), Box<dyn Error>> {
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/context-basic.supergraph.graphql",
        parse_operation(r#"{ users { currency: id txs { amount } } }"#),
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] a               -
      [3] a               [2]

    [2] a
      out
        { users { __typename currency: id _internal_qp_alias_0: currency txs { __typename id } } }

    [3] a   after [2]
      merge
        users.@.txs.@
      in
        ... on Transaction { __typename id }
      out
        ... on Transaction { amount(code: $__context_0) }
      vars
        $__context_0
      context
        $__context_0 = ..|[User]/_internal_qp_alias_0
    ");

    Ok(())
}

#[test]
fn two_root_fields_batch_into_one_fetch() -> Result<(), Box<dyn Error>> {
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/context-basic.supergraph.graphql",
        parse_operation(r#"{ users { txs { amount } } secondUsers { txs { amount } } }"#),
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] a               -
      [4] a               [2]

    [2] a
      out
        { users { ...a } secondUsers { ...a } }
        fragment a on User { __typename currency txs { __typename id } }

    [4] a   after [2]
      alias _e0
        merge
          secondUsers.@.txs.@
        in
          ... on Transaction { __typename id }
        out
          ... on Transaction { amount(code: $__context_1) }
        context
          $__context_1 = ..|[User]/currency
      alias _e1
        merge
          users.@.txs.@
        in
          ... on Transaction { __typename id }
        out
          ... on Transaction { amount(code: $__context_0) }
        context
          $__context_0 = ..|[User]/currency
      vars
        $__context_0, $__context_1
    ");

    Ok(())
}

#[test]
fn multi_type_fetch_does_not_require_contexts_from_mutually_exclusive_ancestors(
) -> Result<(), Box<dyn Error>> {
    let plan = build_query_plan_with_defaults(
        "fixture/tests/context-non-null.supergraph.graphql",
        parse_operation(
            "{ viewer { ... on User { txs { amount } } ... on Other { txs { amount } } } }",
        ),
    )?;
    let plan = plan.to_string();
    assert!(!plan.contains("alias _e"));
    assert!(plan.contains("merge\n    viewer|[User].txs.@\n"), "{plan}");
    assert!(plan.contains("merge\n    viewer|[Other].txs.@\n"), "{plan}");
    Ok(())
}

#[test]
fn context_field_under_skip() -> Result<(), Box<dyn Error>> {
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/context-basic.supergraph.graphql",
        parse_operation(r#"query($skip: Boolean!) { users { txs { amount @skip(if: $skip) } } }"#),
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    variables
      $skip

    fetches
      fetch               after
      [2] a               -
      [3] a               [2]

    [2] a
      out
        { users { __typename currency txs { __typename id } } }

    [3] a   after [2]
      if
        !$skip
      merge
        users.@.txs.@
      in
        ... on Transaction { __typename id }
      out
        ... on Transaction { amount(code: $__context_0) }
      vars
        $__context_0
      context
        $__context_0 = ..|[User]/currency
    ");

    Ok(())
}

#[test]
fn client_alias_on_ancestor_field() -> Result<(), Box<dyn Error>> {
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/context-basic.supergraph.graphql",
        parse_operation(r#"{ renamed: users { txs { amount } } }"#),
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] a               -
      [3] a               [2]

    [2] a
      out
        { renamed: users { __typename currency txs { __typename id } } }

    [3] a   after [2]
      merge
        renamed.@.txs.@
      in
        ... on Transaction { __typename id }
      out
        ... on Transaction { amount(code: $__context_0) }
      vars
        $__context_0
      context
        $__context_0 = ..|[User]/currency
    ");

    Ok(())
}

#[test]
fn cross_subgraph_merges_into_two_fetches() -> Result<(), Box<dyn Error>> {
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/context-cross.supergraph.graphql",
        parse_operation(r#"{ user { lastTransaction { amountInUserCurrency } } }"#),
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] a               -
      [3] b               [2]

    [2] a
      out
        { user { __typename id userCurrency { isoCode } } }

    [3] b   after [2]
      merge
        user
      in
        ... on User { __typename id }
      out
        ... on User {
          __typename
          id
          lastTransaction { __typename id amountInUserCurrency(currencyCode: $__context_0) }
        }
      vars
        $__context_0
      context
        $__context_0 = [User]/userCurrency/isoCode
    ");
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

    insta::assert_snapshot!(plan.to_string(), @r"
    fetches
      fetch               after
      [2] a               -
      [3] a               [2]

    [2] a
      out
        {
          accounts { __typename ... on Member { plan } ... on Guest { plan } wallet { __typename id } }
        }

    [3] a   after [2]
      merge
        accounts.@.wallet
      in
        ... on Wallet { __typename id }
      out
        ... on Wallet { discount(plan: $__context_0) }
      vars
        $__context_0
      context
        $__context_0 = ..|[Member]/plan
        $__context_0 = ..|[Guest]/plan
    ");
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
    insta::assert_snapshot!(plan, @r"
    fetches
      fetch               after
      [2] a               -
      [6] a               [2]
      [4] b               [6]

    [2] a
      out
        { plan { __typename id tier offer { __typename id } } }

    [6] a   after [2]
      merge
        plan.offer
      in
        ... on Offer { __typename id }
      out
        ... on Offer { value(tier: $__context_0) }
      vars
        $__context_0
      context
        $__context_0 = ..|[Plan]/tier

    [4] b   after [6]
      merge
        plan
      in
        ... on Plan { __typename offer { value } id }
      out
        ... on Plan { discount }
    ");
    // the client selects the same contextual field the requirement selects
    let plan = plan_composed(
        "context-with-directives",
        "{ plan { discount offer { value } } }",
    )?;
    insta::assert_snapshot!(plan, @r"
    fetches
      fetch               after
      [2] a               -
      [6] a               [2]
      [4] b               [6]

    [2] a
      out
        { plan { __typename id tier offer { __typename id } } }

    [6] a   after [2]
      merge
        plan.offer
      in
        ... on Offer { __typename id }
      out
        ... on Offer { value(tier: $__context_0) }
      vars
        $__context_0
      context
        $__context_0 = ..|[Plan]/tier

    [4] b   after [6]
      merge
        plan
      in
        ... on Plan { __typename offer { value } id }
      out
        ... on Plan { discount }
    ");
    Ok(())
}

/// `wallet` sits on the `Account` interface, so its `discount` is planned per account type, each
/// with a context variable that reads that type's `plan`. The `Member` call asks for `discount`,
/// the `Guest` one for `a3_discount: discount`. Merged, every wallet would get both, and a
/// `Member` wallet's `a3_discount` would read a `Guest` plan, `null` for it.
#[test]
fn per_type_contextual_fields_under_other_aliases_stay_apart() -> Result<(), Box<dyn Error>> {
    let document = parse_operation(
        r#"
        query ($v15: Boolean! = false, $v4: Boolean!, $v6: Boolean! = false) {
          accounts {
            ... @skip(if: $v15) {
              wallet {
                discount
                ... {
                  a3_discount: discount @skip(if: $v6)
                  discount @skip(if: $v4)
                }
              }
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/context-interface-implementation.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{query_plan}"), @r"
    variables
      $v15: Boolean! = false, $v4: Boolean!, $v6: Boolean! = false

    fetches
      fetch               after
      [2] a               -
      [8] a               [2]

    [2] a
      out
        {
          accounts {
            __typename
            ... on Guest { __typename plan @skip(if: $v15) wallet @skip(if: $v15) { ...a } }
            ... on Member { __typename plan @skip(if: $v15) wallet @skip(if: $v15) { ...a } }
          }
        }
        fragment a on Wallet { __typename id }
      vars
        $v15

    [8] a   after [2]
      alias _e0
        if
          !$v15
        merge
          accounts.@|[Member].wallet
        in
          ... on Wallet { __typename id }
        out
          ... on Wallet {
            ... on Wallet @skip(if: $v4) { ...a }
            ... on Wallet @skip(if: $v6) { a3_discount: discount(plan: $__context_1) }
            ... on Wallet @skip(if: $v15) { ...a }
          }
        context
          $__context_1 = ..|[Member]/plan
      alias _e1
        if
          !$v15
        merge
          accounts.@|[Guest].wallet
        in
          ... on Wallet { __typename id }
        out
          ... on Wallet {
            ... on Wallet @skip(if: $v4) { ...b }
            ... on Wallet @skip(if: $v6) { a3_discount: discount(plan: $__context_0) }
            ... on Wallet @skip(if: $v15) { ...b }
          }
        context
          $__context_0 = ..|[Guest]/plan
      fragments
        fragment a on Wallet { ... on Wallet { discount(plan: $__context_1) } }
        fragment b on Wallet { ... on Wallet { discount(plan: $__context_0) } }
      vars
        $__context_0, $__context_1, $v15, $v4, $v6
    ");

    Ok(())
}

/// `amountInUserCurrency` under `@include` and again under `@skip` reads the same `userCurrency`.
/// Merged into one call, both copies take the same context variable, with one each they'd be one
/// response key with other arguments, which the subgraph rejects.
#[test]
fn same_context_value_under_a_condition_shares_its_variable() -> Result<(), Box<dyn Error>> {
    let document = parse_operation(
        r#"
        query ($a: Boolean!, $b: Boolean!) {
          user {
            lastTransaction {
              ... @include(if: $a) {
                amountInUserCurrency
              }
              ... @skip(if: $b) {
                amountInUserCurrency
              }
            }
          }
        }
        "#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/tests/context-cross.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{query_plan}"), @r"
    variables
      $a: Boolean!, $b: Boolean!

    fetches
      fetch               after
      [2] a               -
      [3] b               [2]

    [2] a
      out
        { user { __typename id userCurrency { isoCode } } }

    [3] b   after [2]
      merge
        user
      in
        ... on User { __typename id }
      out
        ... on User {
          __typename
          id
          lastTransaction {
            ... on Transaction @include(if: $a) {
              __typename
              id
              ... on Transaction @include(if: $a) { ...a }
            }
            ... on Transaction @skip(if: $b) { __typename id ... on Transaction @skip(if: $b) { ...a } }
          }
        }
        fragment a on Transaction { amountInUserCurrency(currencyCode: $__context_0) }
      vars
        $__context_0, $a, $b
      context
        $__context_0 = [User]/userCurrency/isoCode
    ");

    Ok(())
}
