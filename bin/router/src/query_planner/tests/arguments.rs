use crate::query_planner::{
    tests::testkit::{build_query_plan_with_defaults, init_logger},
    utils::parsing::parse_operation,
};
use std::error::Error;

// The "requires-with-argument-conflict" test from the Fed audit
#[test]
fn fed_audit_requires_with_argument_conflict() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          products {
            upc
            name
            shippingEstimate
            shippingEstimateEUR
            isExpensiveCategory
          }
        }"#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/requires-with-argument-conflict.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    fetches
      fetch               after
      [2] b               -
      [4] a               [2]

    [2] b
      out
        {
          products {
            __typename
            upc
            name
            price(currency: "USD")
            weight
            _internal_qp_alias_0: price(currency: "EUR")
            category { averagePrice(currency: "USD") }
          }
        }

    [4] a   after [2]
      alias _e0
        merge
          products.@
        in
          ... on Product { __typename price weight upc category { averagePrice } }
        out
          ... on Product { shippingEstimate isExpensiveCategory }
      alias _e1
        merge
          products.@
        in
          ... on Product { __typename price: _internal_qp_alias_0 weight upc }
        out
          ... on Product { shippingEstimateEUR }
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
            "operation": "{products{__typename upc name price(currency: \"USD\") weight _internal_qp_alias_0: price(currency: \"EUR\") category{averagePrice(currency: \"USD\")}}}"
          },
          {
            "kind": "BatchFetch",
            "serviceName": "a",
            "operationKind": "query",
            "operation": "query($__batch_reps_0:[_Any!]!, $__batch_reps_1:[_Any!]!){_e0: _entities(representations: $__batch_reps_0){...on Product{shippingEstimate isExpensiveCategory}} _e1: _entities(representations: $__batch_reps_1){...on Product{shippingEstimateEUR}}}",
            "entityBatch": {
              "aliases": [
                {
                  "alias": "_e0",
                  "representationsVariableName": "__batch_reps_0",
                  "paths": [
                    [
                      {
                        "Field": "products"
                      },
                      "@"
                    ]
                  ],
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
                          "name": "price"
                        },
                        {
                          "kind": "Field",
                          "name": "weight"
                        },
                        {
                          "kind": "Field",
                          "name": "upc"
                        },
                        {
                          "kind": "Field",
                          "name": "category",
                          "selections": [
                            {
                              "kind": "Field",
                              "name": "averagePrice"
                            }
                          ]
                        }
                      ]
                    }
                  ]
                },
                {
                  "alias": "_e1",
                  "representationsVariableName": "__batch_reps_1",
                  "paths": [
                    [
                      {
                        "Field": "products"
                      },
                      "@"
                    ]
                  ],
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
                          "name": "_internal_qp_alias_0",
                          "alias": "price"
                        },
                        {
                          "kind": "Field",
                          "name": "weight"
                        },
                        {
                          "kind": "Field",
                          "name": "upc"
                        }
                      ]
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

// In this test, `comments(arg: 3)` conflicts with `comments(arg: 1)` in the `feed` field.
// But unlike the other cases in this file, the `comments` field is being queried only after a few other fetches
// are executed, so `comments(arg: 3)` is deeply nested inside the fetch-steps and not a direct descendant of the field that was aliased.
#[test]
fn requires_arguments_deeply_nested_requires() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          feed {
            author {
              id
            }
            comments(limit: 1) {
              id
            }
          }
        }"#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/audit-requires-arguments.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] c               -
      [5] d               [2]
      [6] c               [5]
      [4] d               [6]

    [2] c
      out
        { feed { __typename id } }

    [5] d   after [2]
      merge
        feed.@
      in
        ... on Post { __typename id }
      out
        ... on Post {
          _internal_qp_alias_0: comments(limit: 3) { __typename id }
          comments(limit: 1) { id }
        }

    [6] c   after [5]
      merge
        feed.@._internal_qp_alias_0.@
      in
        ... on Comment { __typename id }
      out
        ... on Comment { authorId }

    [4] d   after [6]
      merge
        feed.@
      in
        ... on Post { __typename comments: _internal_qp_alias_0 { authorId } id }
      out
        ... on Post { author { id } }
    ");

    insta::assert_snapshot!(format!("{}", sonic_rs::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Sequence",
        "nodes": [
          {
            "kind": "Fetch",
            "serviceName": "c",
            "operationKind": "query",
            "operation": "{feed{__typename id}}"
          },
          {
            "kind": "Flatten",
            "path": [
              {
                "Field": "feed"
              },
              "@"
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "d",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Post{_internal_qp_alias_0: comments(limit: 3){__typename id} comments(limit: 1){id}}}}",
              "requires": [
                {
                  "kind": "InlineFragment",
                  "typeCondition": "Post",
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
                "Field": "feed"
              },
              "@",
              {
                "Field": "_internal_qp_alias_0"
              },
              "@"
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "c",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Comment{authorId}}}",
              "requires": [
                {
                  "kind": "InlineFragment",
                  "typeCondition": "Comment",
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
                "Field": "feed"
              },
              "@"
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "d",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Post{author{id}}}}",
              "requires": [
                {
                  "kind": "InlineFragment",
                  "typeCondition": "Post",
                  "selections": [
                    {
                      "kind": "Field",
                      "name": "__typename"
                    },
                    {
                      "kind": "Field",
                      "name": "_internal_qp_alias_0",
                      "selections": [
                        {
                          "kind": "Field",
                          "name": "authorId"
                        }
                      ],
                      "alias": "comments"
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

// Same as "requires_arguments_deeply_nested_requires" but this time with a variable.
// In this one we also make sure that the variable is used in the fetchstep ("variableUsages"). And we ensure it's not merged with other steps.
#[test]
fn requires_arguments_deeply_nested_requires_with_variable() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query ($limit: Int = 1) {
          feed {
            author {
              id
            }
            comments(limit: $limit) {
              id
            }
          }
        }"#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/audit-requires-arguments.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    variables
      $limit: Int = 1

    fetches
      fetch               after
      [2] c               -
      [5] d               [2]
      [6] c               [5]
      [4] d               [6]

    [2] c
      out
        { feed { __typename id } }

    [5] d   after [2]
      merge
        feed.@
      in
        ... on Post { __typename id }
      out
        ... on Post {
          _internal_qp_alias_0: comments(limit: 3) { __typename id }
          comments(limit: $limit) { id }
        }
      vars
        $limit

    [6] c   after [5]
      merge
        feed.@._internal_qp_alias_0.@
      in
        ... on Comment { __typename id }
      out
        ... on Comment { authorId }

    [4] d   after [6]
      merge
        feed.@
      in
        ... on Post { __typename comments: _internal_qp_alias_0 { authorId } id }
      out
        ... on Post { author { id } }
    ");

    insta::assert_snapshot!(format!("{}", sonic_rs::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Sequence",
        "nodes": [
          {
            "kind": "Fetch",
            "serviceName": "c",
            "operationKind": "query",
            "operation": "{feed{__typename id}}"
          },
          {
            "kind": "Flatten",
            "path": [
              {
                "Field": "feed"
              },
              "@"
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "d",
              "variableUsages": [
                "limit"
              ],
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!, $limit:Int=1){_entities(representations: $representations){...on Post{_internal_qp_alias_0: comments(limit: 3){__typename id} comments(limit: $limit){id}}}}",
              "requires": [
                {
                  "kind": "InlineFragment",
                  "typeCondition": "Post",
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
                "Field": "feed"
              },
              "@",
              {
                "Field": "_internal_qp_alias_0"
              },
              "@"
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "c",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Comment{authorId}}}",
              "requires": [
                {
                  "kind": "InlineFragment",
                  "typeCondition": "Comment",
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
                "Field": "feed"
              },
              "@"
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "d",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Post{author{id}}}}",
              "requires": [
                {
                  "kind": "InlineFragment",
                  "typeCondition": "Post",
                  "selections": [
                    {
                      "kind": "Field",
                      "name": "__typename"
                    },
                    {
                      "kind": "Field",
                      "name": "_internal_qp_alias_0",
                      "selections": [
                        {
                          "kind": "Field",
                          "name": "authorId"
                        }
                      ],
                      "alias": "comments"
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

// Same as "requires_arguments_deeply_nested_requires" but this time with a variables and fragments.
#[test]
fn requires_arguments_deeply_nested_requires_with_variables_and_fragments(
) -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query ($limit: Int = 1) {
          feed {
            author {
              id
            }
            ...Foo
            ...Bar
          }
        }

        fragment Foo on Post {
          comments(limit: $limit) {
            id
          }
        }

        fragment Bar on Post {
          comments(limit: $limit) {
            id
          }
        }"#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/audit-requires-arguments.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    variables
      $limit: Int = 1

    fetches
      fetch               after
      [2] c               -
      [5] d               [2]
      [6] c               [5]
      [4] d               [6]

    [2] c
      out
        { feed { __typename id } }

    [5] d   after [2]
      merge
        feed.@
      in
        ... on Post { __typename id }
      out
        ... on Post {
          _internal_qp_alias_0: comments(limit: 3) { __typename id }
          comments(limit: $limit) { id }
        }
      vars
        $limit

    [6] c   after [5]
      merge
        feed.@._internal_qp_alias_0.@
      in
        ... on Comment { __typename id }
      out
        ... on Comment { authorId }

    [4] d   after [6]
      merge
        feed.@
      in
        ... on Post { __typename comments: _internal_qp_alias_0 { authorId } id }
      out
        ... on Post { author { id } }
    ");

    insta::assert_snapshot!(format!("{}", sonic_rs::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Sequence",
        "nodes": [
          {
            "kind": "Fetch",
            "serviceName": "c",
            "operationKind": "query",
            "operation": "{feed{__typename id}}"
          },
          {
            "kind": "Flatten",
            "path": [
              {
                "Field": "feed"
              },
              "@"
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "d",
              "variableUsages": [
                "limit"
              ],
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!, $limit:Int=1){_entities(representations: $representations){...on Post{_internal_qp_alias_0: comments(limit: 3){__typename id} comments(limit: $limit){id}}}}",
              "requires": [
                {
                  "kind": "InlineFragment",
                  "typeCondition": "Post",
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
                "Field": "feed"
              },
              "@",
              {
                "Field": "_internal_qp_alias_0"
              },
              "@"
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "c",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Comment{authorId}}}",
              "requires": [
                {
                  "kind": "InlineFragment",
                  "typeCondition": "Comment",
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
                "Field": "feed"
              },
              "@"
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "d",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Post{author{id}}}}",
              "requires": [
                {
                  "kind": "InlineFragment",
                  "typeCondition": "Post",
                  "selections": [
                    {
                      "kind": "Field",
                      "name": "__typename"
                    },
                    {
                      "kind": "Field",
                      "name": "_internal_qp_alias_0",
                      "selections": [
                        {
                          "kind": "Field",
                          "name": "authorId"
                        }
                      ],
                      "alias": "comments"
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

// In this test, each one of the queries fields has a requirement with different arguments.
// This leads to a conflict when the fetch steps are built.
// The parent fetch can be grouped, but only if we alias the the output fields.
// Aliasing means that the child fetches cannot be grouped, and they need input_rewrite in order to be executed correctly.
#[test]
fn multiple_requires_with_args_that_conflicts() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        {
          test {
            id
            fieldWithRequiresAndArgs # requires(fields: "otherField(arg: 2)")
            anotherWithRequiresAndArgs # requires(fields: "otherField(arg: 3)")
          }
        }"#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/simple-requires-args.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] a               -
      [4] b               [2]
      [5] a               [4]

    [2] a
      out
        { test { __typename id } }

    [4] b   after [2]
      merge
        test
      in
        ... on Test { __typename id }
      out
        ... on Test { otherField(arg: 2) _internal_qp_alias_0: otherField(arg: 3) }

    [5] a   after [4]
      alias _e0
        merge
          test
        in
          ... on Test { __typename otherField: _internal_qp_alias_0 id }
        out
          ... on Test { anotherWithRequiresAndArgs }
      alias _e1
        merge
          test
        in
          ... on Test { __typename otherField id }
        out
          ... on Test { fieldWithRequiresAndArgs }
    ");

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
            "operation": "{test{__typename id}}"
          },
          {
            "kind": "Flatten",
            "path": [
              {
                "Field": "test"
              }
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "b",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Test{otherField(arg: 2) _internal_qp_alias_0: otherField(arg: 3)}}}",
              "requires": [
                {
                  "kind": "InlineFragment",
                  "typeCondition": "Test",
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
            "kind": "BatchFetch",
            "serviceName": "a",
            "operationKind": "query",
            "operation": "query($__batch_reps_0:[_Any!]!, $__batch_reps_1:[_Any!]!){_e0: _entities(representations: $__batch_reps_0){...on Test{anotherWithRequiresAndArgs}} _e1: _entities(representations: $__batch_reps_1){...on Test{fieldWithRequiresAndArgs}}}",
            "entityBatch": {
              "aliases": [
                {
                  "alias": "_e0",
                  "representationsVariableName": "__batch_reps_0",
                  "paths": [
                    [
                      {
                        "Field": "test"
                      }
                    ]
                  ],
                  "requires": [
                    {
                      "kind": "InlineFragment",
                      "typeCondition": "Test",
                      "selections": [
                        {
                          "kind": "Field",
                          "name": "__typename"
                        },
                        {
                          "kind": "Field",
                          "name": "_internal_qp_alias_0",
                          "alias": "otherField"
                        },
                        {
                          "kind": "Field",
                          "name": "id"
                        }
                      ]
                    }
                  ]
                },
                {
                  "alias": "_e1",
                  "representationsVariableName": "__batch_reps_1",
                  "paths": [
                    [
                      {
                        "Field": "test"
                      }
                    ]
                  ],
                  "requires": [
                    {
                      "kind": "InlineFragment",
                      "typeCondition": "Test",
                      "selections": [
                        {
                          "kind": "Field",
                          "name": "__typename"
                        },
                        {
                          "kind": "Field",
                          "name": "otherField"
                        },
                        {
                          "kind": "Field",
                          "name": "id"
                        }
                      ]
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

// In this test, we have a user-request field, along with fields that have requirements.
// All use the same "otherField" to get the actual data, and they conflict if tried to be grouped.
// We make sure that "otherField(arg: 1)" remains as-is, while other conflicting fields (otherField(arg: 2) and otherField(arg: 3)) are being aliased.
// Each alias yields a input-rewrite, to make sure following fetchsteps can use it correctly.
#[test]
fn multiple_plain_field_and_requires_with_args_that_conflicts() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        {
          test {
            id
            fieldWithRequiresAndArgs # requires(fields: "otherField(arg: 2)")
            anotherWithRequiresAndArgs # requires(fields: "otherField(arg: 3)")
            otherField(arg: 1)
          }
        }"#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/simple-requires-args.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] a               -
      [6] b               [2]
      [3] a               [6]

    [2] a
      out
        { test { __typename id } }

    [6] b   after [2]
      merge
        test
      in
        ... on Test { __typename id }
      out
        ... on Test {
          otherField(arg: 1)
          _internal_qp_alias_0: otherField(arg: 2)
          _internal_qp_alias_1: otherField(arg: 3)
        }

    [3] a   after [6]
      alias _e0
        merge
          test
        in
          ... on Test { __typename otherField: _internal_qp_alias_0 id }
        out
          ... on Test { fieldWithRequiresAndArgs }
      alias _e1
        merge
          test
        in
          ... on Test { __typename otherField: _internal_qp_alias_1 id }
        out
          ... on Test { anotherWithRequiresAndArgs }
    ");

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
            "operation": "{test{__typename id}}"
          },
          {
            "kind": "Flatten",
            "path": [
              {
                "Field": "test"
              }
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "b",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Test{otherField(arg: 1) _internal_qp_alias_0: otherField(arg: 2) _internal_qp_alias_1: otherField(arg: 3)}}}",
              "requires": [
                {
                  "kind": "InlineFragment",
                  "typeCondition": "Test",
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
            "kind": "BatchFetch",
            "serviceName": "a",
            "operationKind": "query",
            "operation": "query($__batch_reps_0:[_Any!]!, $__batch_reps_1:[_Any!]!){_e0: _entities(representations: $__batch_reps_0){...on Test{fieldWithRequiresAndArgs}} _e1: _entities(representations: $__batch_reps_1){...on Test{anotherWithRequiresAndArgs}}}",
            "entityBatch": {
              "aliases": [
                {
                  "alias": "_e0",
                  "representationsVariableName": "__batch_reps_0",
                  "paths": [
                    [
                      {
                        "Field": "test"
                      }
                    ]
                  ],
                  "requires": [
                    {
                      "kind": "InlineFragment",
                      "typeCondition": "Test",
                      "selections": [
                        {
                          "kind": "Field",
                          "name": "__typename"
                        },
                        {
                          "kind": "Field",
                          "name": "_internal_qp_alias_0",
                          "alias": "otherField"
                        },
                        {
                          "kind": "Field",
                          "name": "id"
                        }
                      ]
                    }
                  ]
                },
                {
                  "alias": "_e1",
                  "representationsVariableName": "__batch_reps_1",
                  "paths": [
                    [
                      {
                        "Field": "test"
                      }
                    ]
                  ],
                  "requires": [
                    {
                      "kind": "InlineFragment",
                      "typeCondition": "Test",
                      "selections": [
                        {
                          "kind": "Field",
                          "name": "__typename"
                        },
                        {
                          "kind": "Field",
                          "name": "_internal_qp_alias_1",
                          "alias": "otherField"
                        },
                        {
                          "kind": "Field",
                          "name": "id"
                        }
                      ]
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

// In this test, we have a user-request field, along with fields that have requirements.
// All use the same "otherField" arguments, to get the actual data, so there is no conflict.
// Result is expected to be a single "otherField" field with the correct arguments, no aliases or rewrites are needed.
#[test]
fn multiple_plain_field_and_requires_with_args_that_does_not_conflicts_should_merge(
) -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        {
          test {
            id
            fieldWithRequiresAndArgs # requires(fields: "otherField(arg: 2)")
            otherField(arg: 2)
          }
        }"#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/simple-requires-args.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] a               -
      [5] b               [2]
      [3] a               [5]

    [2] a
      out
        { test { __typename id } }

    [5] b   after [2]
      merge
        test
      in
        ... on Test { __typename id }
      out
        ... on Test { otherField(arg: 2) }

    [3] a   after [5]
      merge
        test
      in
        ... on Test { __typename otherField id }
      out
        ... on Test { fieldWithRequiresAndArgs }
    ");

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
            "operation": "{test{__typename id}}"
          },
          {
            "kind": "Flatten",
            "path": [
              {
                "Field": "test"
              }
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "b",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Test{otherField(arg: 2)}}}",
              "requires": [
                {
                  "kind": "InlineFragment",
                  "typeCondition": "Test",
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
                "Field": "test"
              }
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "a",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Test{fieldWithRequiresAndArgs}}}",
              "requires": [
                {
                  "kind": "InlineFragment",
                  "typeCondition": "Test",
                  "selections": [
                    {
                      "kind": "Field",
                      "name": "__typename"
                    },
                    {
                      "kind": "Field",
                      "name": "otherField"
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
fn simple_requires_arguments() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        {
          test {
            id
            fieldWithRequiresAndArgs
          }
        }"#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/simple-requires-args.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] a               -
      [4] b               [2]
      [3] a               [4]

    [2] a
      out
        { test { __typename id } }

    [4] b   after [2]
      merge
        test
      in
        ... on Test { __typename id }
      out
        ... on Test { otherField(arg: 2) }

    [3] a   after [4]
      merge
        test
      in
        ... on Test { __typename otherField id }
      out
        ... on Test { fieldWithRequiresAndArgs }
    ");

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
            "operation": "{test{__typename id}}"
          },
          {
            "kind": "Flatten",
            "path": [
              {
                "Field": "test"
              }
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "b",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Test{otherField(arg: 2)}}}",
              "requires": [
                {
                  "kind": "InlineFragment",
                  "typeCondition": "Test",
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
                "Field": "test"
              }
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "a",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Test{fieldWithRequiresAndArgs}}}",
              "requires": [
                {
                  "kind": "InlineFragment",
                  "typeCondition": "Test",
                  "selections": [
                    {
                      "kind": "Field",
                      "name": "__typename"
                    },
                    {
                      "kind": "Field",
                      "name": "otherField"
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
fn requires_with_arguments() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          feed {
            author {
              id
            }
          }
        }"#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/arguments-requires.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] a               -
      [5] b               [2]
      [6] a               [5]
      [4] b               [6]

    [2] a
      out
        { feed { __typename id } }

    [5] b   after [2]
      merge
        feed.@
      in
        ... on Post { __typename id }
      out
        ... on Post { comments(limit: 3) { __typename id } }

    [6] a   after [5]
      merge
        feed.@.comments.@
      in
        ... on Comment { __typename id }
      out
        ... on Comment { somethingElse }

    [4] b   after [6]
      merge
        feed.@
      in
        ... on Post { __typename comments { somethingElse } id }
      out
        ... on Post { author { id } }
    ");

    Ok(())
}

#[test]
fn arguments_in_different_levels() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          album(id: "5") {
            albumType
            name
            genres
            tracks(limit: 5, offset: 10) {
              edges {
                node {
                  name
                }
              }
            }
          }

        }"#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/spotify-supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    fetches
      fetch               after
      [2] spotify         -

    [2] spotify
      out
        {
          album(id: "5") {
            albumType
            name
            genres
            tracks(limit: 5, offset: 10) { edges { node { name } } }
          }
        }
    "#);

    insta::assert_snapshot!(format!("{}", sonic_rs::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Fetch",
        "serviceName": "spotify",
        "operationKind": "query",
        "operation": "{album(id: \"5\"){albumType name genres tracks(limit: 5, offset: 10){edges{node{name}}}}}"
      }
    }
    "#);

    Ok(())
}

#[test]
fn arguments_and_variables() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query test($id: ID!, $limit: Int) {
          album(id: $id) {
            albumType
            name
            genres
            tracks(limit: $limit, offset: 10) {
              edges {
                node {
                  name
                }
              }
            }
          }

        }"#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/spotify-supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    variables
      $id: ID!, $limit: Int

    fetches
      fetch               after
      [2] spotify         -

    [2] spotify
      out
        {
          album(id: $id) {
            albumType
            name
            genres
            tracks(limit: $limit, offset: 10) { edges { node { name } } }
          }
        }
      vars
        $id, $limit
    ");
    insta::assert_snapshot!(format!("{}", sonic_rs::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Fetch",
        "serviceName": "spotify",
        "variableUsages": [
          "id",
          "limit"
        ],
        "operationKind": "query",
        "operation": "query($id:ID!, $limit:Int){album(id: $id){albumType name genres tracks(limit: $limit, offset: 10){edges{node{name}}}}}"
      }
    }
    "#);
    Ok(())
}

#[test]
fn arguments_with_aliases() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          firstProduct: productFromD(id: "1") {
            id
            name
            category {
              id
              name
              details
            }
          }
          secondProduct: productFromD(id: "2") {
            id
            name
            category {
              id
              name
              details
            }
          }
        }"#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/parent-entity-call-complex.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    fetches
      fetch               after
      [2] d               -
      [8] a               [2]
      [6] b               [2]
      [7] c               [6]

    [2] d
      out
        { firstProduct: productFromD(id: "1") { ...a } secondProduct: productFromD(id: "2") { ...a } }
        fragment a on Product { __typename id name }

    [8] a   after [2]
      alias _e0
        merge
          secondProduct
          firstProduct
        in
          ... on Product { __typename id }
        out
          ... on Product { category { details } }

    [6] b   after [2]
      alias _e0
        merge
          secondProduct
          firstProduct
        in
          ... on Product { __typename id }
        out
          ... on Product { category { __typename id } }

    [7] c   after [6]
      alias _e0
        merge
          secondProduct.category
          firstProduct.category
        in
          ... on Category { __typename id }
        out
          ... on Category { name }
    "#);
    Ok(())
}

#[test]
fn arguments_variables_mixed() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query test($secondProductId: ID!) {
          firstProduct: productFromD(id: "1") {
            id
            name
            category {
              id
              name
              details
            }
          }
          secondProduct: productFromD(id: $secondProductId) {
            id
            name
            category {
              id
              name
              details
            }
          }
        }"#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/parent-entity-call-complex.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    variables
      $secondProductId: ID!

    fetches
      fetch               after
      [2] d               -
      [8] a               [2]
      [6] b               [2]
      [7] c               [6]

    [2] d
      out
        {
          firstProduct: productFromD(id: "1") { ...a }
          secondProduct: productFromD(id: $secondProductId) { ...a }
        }
        fragment a on Product { __typename id name }
      vars
        $secondProductId

    [8] a   after [2]
      alias _e0
        merge
          secondProduct
          firstProduct
        in
          ... on Product { __typename id }
        out
          ... on Product { category { details } }

    [6] b   after [2]
      alias _e0
        merge
          secondProduct
          firstProduct
        in
          ... on Product { __typename id }
        out
          ... on Product { category { __typename id } }

    [7] c   after [6]
      alias _e0
        merge
          secondProduct.category
          firstProduct.category
        in
          ... on Category { __typename id }
        out
          ... on Category { name }
    "#);

    insta::assert_snapshot!(format!("{}", sonic_rs::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Sequence",
        "nodes": [
          {
            "kind": "Fetch",
            "serviceName": "d",
            "variableUsages": [
              "secondProductId"
            ],
            "operationKind": "query",
            "operation": "query($secondProductId:ID!){firstProduct: productFromD(id: \"1\"){...a} secondProduct: productFromD(id: $secondProductId){...a}}\n\nfragment a on Product {__typename id name}\n"
          },
          {
            "kind": "Parallel",
            "nodes": [
              {
                "kind": "BatchFetch",
                "serviceName": "a",
                "operationKind": "query",
                "operation": "query($__batch_reps_0:[_Any!]!){_e0: _entities(representations: $__batch_reps_0){...on Product{category{details}}}}",
                "entityBatch": {
                  "aliases": [
                    {
                      "alias": "_e0",
                      "representationsVariableName": "__batch_reps_0",
                      "paths": [
                        [
                          {
                            "Field": "secondProduct"
                          }
                        ],
                        [
                          {
                            "Field": "firstProduct"
                          }
                        ]
                      ],
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
                      ]
                    }
                  ]
                }
              },
              {
                "kind": "BatchFetch",
                "serviceName": "b",
                "operationKind": "query",
                "operation": "query($__batch_reps_0:[_Any!]!){_e0: _entities(representations: $__batch_reps_0){...on Product{category{__typename id}}}}",
                "entityBatch": {
                  "aliases": [
                    {
                      "alias": "_e0",
                      "representationsVariableName": "__batch_reps_0",
                      "paths": [
                        [
                          {
                            "Field": "secondProduct"
                          }
                        ],
                        [
                          {
                            "Field": "firstProduct"
                          }
                        ]
                      ],
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
                      ]
                    }
                  ]
                }
              }
            ]
          },
          {
            "kind": "BatchFetch",
            "serviceName": "c",
            "operationKind": "query",
            "operation": "query($__batch_reps_0:[_Any!]!){_e0: _entities(representations: $__batch_reps_0){...on Category{name}}}",
            "entityBatch": {
              "aliases": [
                {
                  "alias": "_e0",
                  "representationsVariableName": "__batch_reps_0",
                  "paths": [
                    [
                      {
                        "Field": "secondProduct"
                      },
                      {
                        "Field": "category"
                      }
                    ],
                    [
                      {
                        "Field": "firstProduct"
                      },
                      {
                        "Field": "category"
                      }
                    ]
                  ],
                  "requires": [
                    {
                      "kind": "InlineFragment",
                      "typeCondition": "Category",
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
              ]
            }
          }
        ]
      }
    }
    "#);

    Ok(())
}

/// Same idea as `fed_audit_requires_with_argument_conflict`, but the `Order` comes out of an
/// `_entities` fetch. `usd` needs `price(currency: "USD")` and `eur` needs
/// `price(currency: "EUR")`, both from `orders`, so the second one is sent as
/// `_internal_qp_alias_0`.
///
/// `eur`'s `_entities` call has to read `price: _internal_qp_alias_0`, like in the audit case.
/// We used to send plain `price` - the USD value - to both, because the alias was recorded
/// from `orders.@` and looked up as if it started at the `user` fetch's root. See the matching
/// e2e test in `e2e/src/issues/mod.rs`.
#[test]
fn requires_with_argument_conflict_after_entity_hop() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        {
          user {
            orders {
              usd
              eur
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/requires-argument-conflict-after-hop.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    fetches
      fetch               after
      [2] users           -
      [3] orders          [2]
      [5] catalog         [3]

    [2] users
      out
        { user { __typename id } }

    [3] orders   after [2]
      merge
        user
      in
        ... on User { __typename id }
      out
        ... on User {
          orders { __typename id price(currency: "USD") _internal_qp_alias_0: price(currency: "EUR") }
        }

    [5] catalog   after [3]
      alias _e0
        merge
          user.orders.@
        in
          ... on Order { __typename price id }
        out
          ... on Order { usd }
      alias _e1
        merge
          user.orders.@
        in
          ... on Order { __typename price: _internal_qp_alias_0 id }
        out
          ... on Order { eur }
    "#);

    Ok(())
}

/// https://github.com/graphql-hive/router/issues/1829
///
/// `expensiveUsdCategory` needs `category { averagePrice(currency: "USD") }` and
/// `expensiveEurCategory` needs `category { averagePrice(currency: "EUR") }`. The EUR one is
/// `_internal_qp_alias_0` inside `category`, so the two calls to `a` can't share one input.
#[test]
fn requires_with_nested_argument_conflict() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          products {
            upc
            usdAverage: category { averagePrice(currency: "USD") }
            expensiveUsdCategory
            expensiveEurCategory
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/requires-nested-argument-conflict.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    fetches
      fetch               after
      [2] b               -
      [4] a               [2]

    [2] b
      out
        {
          products {
            __typename
            upc
            usdAverage: category { averagePrice(currency: "USD") }
            category {
              averagePrice(currency: "USD")
              _internal_qp_alias_0: averagePrice(currency: "EUR")
            }
          }
        }

    [4] a   after [2]
      alias _e0
        merge
          products.@
        in
          ... on Product { __typename category { averagePrice } upc }
        out
          ... on Product { expensiveUsdCategory }
      alias _e1
        merge
          products.@
        in
          ... on Product { __typename category { averagePrice: _internal_qp_alias_0 } upc }
        out
          ... on Product { expensiveEurCategory }
    "#);

    Ok(())
}

/// `expensiveUsdCategory` and `cheapUsdCategory` both need `category { averagePrice(currency: "USD") }`,
/// so the two calls to `a` can share one input and go out as one.
#[test]
fn requires_with_same_nested_arguments_share_a_call() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          products {
            expensiveUsdCategory
            cheapUsdCategory
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/requires-nested-argument-conflict.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    fetches
      fetch               after
      [2] b               -
      [4] a               [2]

    [2] b
      out
        { products { __typename upc category { averagePrice(currency: "USD") } } }

    [4] a   after [2]
      merge
        products.@
      in
        ... on Product { __typename category { averagePrice } upc }
      out
        ... on Product { expensiveUsdCategory cheapUsdCategory }
    "#);

    Ok(())
}
