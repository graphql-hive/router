use crate::{
    tests::testkit::{build_query_plan, init_logger},
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
    let query_plan = build_query_plan(
        "fixture/tests/requires-with-argument-conflict.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "b") {
          {
            products {
              __typename
              upc
              name
              price(currency: "USD")
              weight
              _internal_qp_alias_0: price(currency: "EUR")
              category {
                averagePrice(currency: "USD")
              }
            }
          }
        },
        Parallel {
          Flatten(path: "products.@") {
            Fetch(service: "a") {
              {
                ... on Product {
                  __typename
                  category {
                    averagePrice
                  }
                  upc
                  price: _internal_qp_alias_0
                  weight
                }
              } =>
              {
                ... on Product {
                  isExpensiveCategory
                  shippingEstimateEUR
                }
              }
            },
          },
          Flatten(path: "products.@") {
            Fetch(service: "a") {
              {
                ... on Product {
                  __typename
                  price
                  weight
                  upc
                }
              } =>
              {
                ... on Product {
                  shippingEstimate
                }
              }
            },
          },
        },
      },
    },
    "#);

    insta::assert_snapshot!(format!("{}", serde_json::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Sequence",
        "nodes": [
          {
            "kind": "Fetch",
            "serviceName": "b",
            "operationKind": "query",
            "operation": "query{products{__typename upc name price(currency: \"USD\") weight _internal_qp_alias_0: price(currency: \"EUR\") category{averagePrice(currency: \"USD\")}}}"
          },
          {
            "kind": "Parallel",
            "nodes": [
              {
                "kind": "Flatten",
                "path": [
                  {
                    "Field": "products"
                  },
                  "List"
                ],
                "node": {
                  "kind": "Fetch",
                  "serviceName": "a",
                  "operationKind": "query",
                  "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Product{isExpensiveCategory shippingEstimateEUR}}}",
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
                          "name": "category",
                          "selections": [
                            {
                              "kind": "Field",
                              "name": "averagePrice"
                            }
                          ]
                        },
                        {
                          "kind": "Field",
                          "name": "upc"
                        },
                        {
                          "kind": "Field",
                          "name": "_internal_qp_alias_0",
                          "alias": "price"
                        },
                        {
                          "kind": "Field",
                          "name": "weight"
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
                    "Field": "products"
                  },
                  "List"
                ],
                "node": {
                  "kind": "Fetch",
                  "serviceName": "a",
                  "operationKind": "query",
                  "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Product{shippingEstimate}}}",
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
                        }
                      ]
                    }
                  ]
                }
              }
            ]
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
    let query_plan = build_query_plan(
        "fixture/tests/audit-requires-arguments.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "c") {
          {
            feed {
              __typename
              id
            }
          }
        },
        Flatten(path: "feed.@") {
          Fetch(service: "d") {
            {
              ... on Post {
                __typename
                id
              }
            } =>
            {
              ... on Post {
                _internal_qp_alias_0: comments(limit: 3) {
                  __typename
                  id
                }
                comments(limit: 1) {
                  id
                }
              }
            }
          },
        },
        Flatten(path: "feed.@._internal_qp_alias_0.@") {
          Fetch(service: "c") {
            {
              ... on Comment {
                __typename
                id
              }
            } =>
            {
              ... on Comment {
                authorId
              }
            }
          },
        },
        Flatten(path: "feed.@") {
          Fetch(service: "d") {
            {
              ... on Post {
                __typename
                comments: _internal_qp_alias_0 {
                  authorId
                }
                id
              }
            } =>
            {
              ... on Post {
                author {
                  id
                }
              }
            }
          },
        },
      },
    },
    "#);

    insta::assert_snapshot!(format!("{}", serde_json::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Sequence",
        "nodes": [
          {
            "kind": "Fetch",
            "serviceName": "c",
            "operationKind": "query",
            "operation": "query{feed{__typename id}}"
          },
          {
            "kind": "Flatten",
            "path": [
              {
                "Field": "feed"
              },
              "List"
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
              "List",
              {
                "Field": "_internal_qp_alias_0"
              },
              "List"
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
              "List"
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
    let query_plan = build_query_plan(
        "fixture/tests/audit-requires-arguments.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "c") {
          {
            feed {
              __typename
              id
            }
          }
        },
        Flatten(path: "feed.@") {
          Fetch(service: "d") {
            {
              ... on Post {
                __typename
                id
              }
            } =>
            ($limit:Int=1) {
              ... on Post {
                _internal_qp_alias_0: comments(limit: 3) {
                  __typename
                  id
                }
                comments(limit: $limit) {
                  id
                }
              }
            }
          },
        },
        Flatten(path: "feed.@._internal_qp_alias_0.@") {
          Fetch(service: "c") {
            {
              ... on Comment {
                __typename
                id
              }
            } =>
            {
              ... on Comment {
                authorId
              }
            }
          },
        },
        Flatten(path: "feed.@") {
          Fetch(service: "d") {
            {
              ... on Post {
                __typename
                comments: _internal_qp_alias_0 {
                  authorId
                }
                id
              }
            } =>
            {
              ... on Post {
                author {
                  id
                }
              }
            }
          },
        },
      },
    },
    "#);

    insta::assert_snapshot!(format!("{}", serde_json::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Sequence",
        "nodes": [
          {
            "kind": "Fetch",
            "serviceName": "c",
            "operationKind": "query",
            "operation": "query{feed{__typename id}}"
          },
          {
            "kind": "Flatten",
            "path": [
              {
                "Field": "feed"
              },
              "List"
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
              "List",
              {
                "Field": "_internal_qp_alias_0"
              },
              "List"
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
              "List"
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
    let query_plan = build_query_plan(
        "fixture/tests/audit-requires-arguments.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "c") {
          {
            feed {
              __typename
              id
            }
          }
        },
        Flatten(path: "feed.@") {
          Fetch(service: "d") {
            {
              ... on Post {
                __typename
                id
              }
            } =>
            ($limit:Int=1) {
              ... on Post {
                _internal_qp_alias_0: comments(limit: 3) {
                  __typename
                  id
                }
                comments(limit: $limit) {
                  id
                }
              }
            }
          },
        },
        Flatten(path: "feed.@._internal_qp_alias_0.@") {
          Fetch(service: "c") {
            {
              ... on Comment {
                __typename
                id
              }
            } =>
            {
              ... on Comment {
                authorId
              }
            }
          },
        },
        Flatten(path: "feed.@") {
          Fetch(service: "d") {
            {
              ... on Post {
                __typename
                comments: _internal_qp_alias_0 {
                  authorId
                }
                id
              }
            } =>
            {
              ... on Post {
                author {
                  id
                }
              }
            }
          },
        },
      },
    },
    "#);

    insta::assert_snapshot!(format!("{}", serde_json::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Sequence",
        "nodes": [
          {
            "kind": "Fetch",
            "serviceName": "c",
            "operationKind": "query",
            "operation": "query{feed{__typename id}}"
          },
          {
            "kind": "Flatten",
            "path": [
              {
                "Field": "feed"
              },
              "List"
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
              "List",
              {
                "Field": "_internal_qp_alias_0"
              },
              "List"
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
              "List"
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
    let query_plan = build_query_plan(
        "fixture/tests/simple-requires-args.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          {
            test {
              id
              __typename
            }
          }
        },
        Flatten(path: "test") {
          Fetch(service: "b") {
            {
              ... on Test {
                __typename
                id
              }
            } =>
            {
              ... on Test {
                otherField(arg: 2)
                _internal_qp_alias_0: otherField(arg: 3)
              }
            }
          },
        },
        Parallel {
          Flatten(path: "test") {
            Fetch(service: "a") {
              {
                ... on Test {
                  __typename
                  otherField: _internal_qp_alias_0
                  id
                }
              } =>
              {
                ... on Test {
                  anotherWithRequiresAndArgs
                }
              }
            },
          },
          Flatten(path: "test") {
            Fetch(service: "a") {
              {
                ... on Test {
                  __typename
                  otherField
                  id
                }
              } =>
              {
                ... on Test {
                  fieldWithRequiresAndArgs
                }
              }
            },
          },
        },
      },
    },
    "#);

    insta::assert_snapshot!(format!("{}", serde_json::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Sequence",
        "nodes": [
          {
            "kind": "Fetch",
            "serviceName": "a",
            "operationKind": "query",
            "operation": "query{test{id __typename}}"
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
            "kind": "Parallel",
            "nodes": [
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
                  "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Test{anotherWithRequiresAndArgs}}}",
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
    let query_plan = build_query_plan(
        "fixture/tests/simple-requires-args.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          {
            test {
              __typename
              id
            }
          }
        },
        Flatten(path: "test") {
          Fetch(service: "b") {
            {
              ... on Test {
                __typename
                id
              }
            } =>
            {
              ... on Test {
                _internal_qp_alias_1: otherField(arg: 2)
                _internal_qp_alias_0: otherField(arg: 3)
                otherField(arg: 1)
              }
            }
          },
        },
        Parallel {
          Flatten(path: "test") {
            Fetch(service: "a") {
              {
                ... on Test {
                  __typename
                  otherField: _internal_qp_alias_0
                  id
                }
              } =>
              {
                ... on Test {
                  anotherWithRequiresAndArgs
                }
              }
            },
          },
          Flatten(path: "test") {
            Fetch(service: "a") {
              {
                ... on Test {
                  __typename
                  otherField: _internal_qp_alias_1
                  id
                }
              } =>
              {
                ... on Test {
                  fieldWithRequiresAndArgs
                }
              }
            },
          },
        },
      },
    },
    "#);

    insta::assert_snapshot!(format!("{}", serde_json::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Sequence",
        "nodes": [
          {
            "kind": "Fetch",
            "serviceName": "a",
            "operationKind": "query",
            "operation": "query{test{__typename id}}"
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
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Test{_internal_qp_alias_1: otherField(arg: 2) _internal_qp_alias_0: otherField(arg: 3) otherField(arg: 1)}}}",
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
            "kind": "Parallel",
            "nodes": [
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
                  "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Test{anotherWithRequiresAndArgs}}}",
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
              }
            ]
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
    let query_plan = build_query_plan(
        "fixture/tests/simple-requires-args.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          {
            test {
              __typename
              id
            }
          }
        },
        Flatten(path: "test") {
          Fetch(service: "b") {
            {
              ... on Test {
                __typename
                id
              }
            } =>
            {
              ... on Test {
                otherField(arg: 2)
              }
            }
          },
        },
        Flatten(path: "test") {
          Fetch(service: "a") {
            {
              ... on Test {
                __typename
                otherField
                id
              }
            } =>
            {
              ... on Test {
                fieldWithRequiresAndArgs
              }
            }
          },
        },
      },
    },
    "#);

    insta::assert_snapshot!(format!("{}", serde_json::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Sequence",
        "nodes": [
          {
            "kind": "Fetch",
            "serviceName": "a",
            "operationKind": "query",
            "operation": "query{test{__typename id}}"
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
    let query_plan = build_query_plan(
        "fixture/tests/simple-requires-args.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          {
            test {
              id
              __typename
            }
          }
        },
        Flatten(path: "test") {
          Fetch(service: "b") {
            {
              ... on Test {
                __typename
                id
              }
            } =>
            {
              ... on Test {
                otherField(arg: 2)
              }
            }
          },
        },
        Flatten(path: "test") {
          Fetch(service: "a") {
            {
              ... on Test {
                __typename
                otherField
                id
              }
            } =>
            {
              ... on Test {
                fieldWithRequiresAndArgs
              }
            }
          },
        },
      },
    },
    "#);

    insta::assert_snapshot!(format!("{}", serde_json::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Sequence",
        "nodes": [
          {
            "kind": "Fetch",
            "serviceName": "a",
            "operationKind": "query",
            "operation": "query{test{id __typename}}"
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
    let query_plan = build_query_plan(
        "fixture/tests/arguments-requires.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          {
            feed {
              __typename
              id
            }
          }
        },
        Flatten(path: "feed.@") {
          Fetch(service: "b") {
            {
              ... on Post {
                __typename
                id
              }
            } =>
            {
              ... on Post {
                comments(limit: 3) {
                  __typename
                  id
                }
              }
            }
          },
        },
        Flatten(path: "feed.@.comments.@") {
          Fetch(service: "a") {
            {
              ... on Comment {
                __typename
                id
              }
            } =>
            {
              ... on Comment {
                somethingElse
              }
            }
          },
        },
        Flatten(path: "feed.@") {
          Fetch(service: "b") {
            {
              ... on Post {
                __typename
                comments {
                  somethingElse
                }
                id
              }
            } =>
            {
              ... on Post {
                author {
                  id
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
    let query_plan = build_query_plan("fixture/spotify-supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "spotify") {
        {
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
        }
      },
    },
    "#);

    insta::assert_snapshot!(format!("{}", serde_json::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Fetch",
        "serviceName": "spotify",
        "operationKind": "query",
        "operation": "query{album(id: \"5\"){albumType name genres tracks(limit: 5, offset: 10){edges{node{name}}}}}"
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
    let query_plan = build_query_plan("fixture/spotify-supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "spotify") {
        query ($id:ID!,$limit:Int) {
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
        }
      },
    },
    "#);
    insta::assert_snapshot!(format!("{}", serde_json::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
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
    let query_plan = build_query_plan(
        "fixture/tests/parent-entity-call-complex.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "d") {
          {
            firstProduct: productFromD(id: "1") {
    ...a        }
            secondProduct: productFromD(id: "2") {
    ...a        }
          }
          fragment a on Product {
            __typename
            id
            name
          }
        },
        Parallel {
          Flatten(path: "secondProduct") {
            Fetch(service: "a") {
              {
                ... on Product {
                  __typename
                  id
                }
              } =>
              {
                ... on Product {
                  category {
                    details
                  }
                }
              }
            },
          },
          Flatten(path: "secondProduct") {
            Fetch(service: "b") {
              {
                ... on Product {
                  __typename
                  id
                }
              } =>
              {
                ... on Product {
                  category {
                    __typename
                    id
                  }
                }
              }
            },
          },
          Flatten(path: "firstProduct") {
            Fetch(service: "a") {
              {
                ... on Product {
                  __typename
                  id
                }
              } =>
              {
                ... on Product {
                  category {
                    details
                  }
                }
              }
            },
          },
          Flatten(path: "firstProduct") {
            Fetch(service: "b") {
              {
                ... on Product {
                  __typename
                  id
                }
              } =>
              {
                ... on Product {
                  category {
                    __typename
                    id
                  }
                }
              }
            },
          },
        },
        Parallel {
          Flatten(path: "secondProduct.category") {
            Fetch(service: "c") {
              {
                ... on Category {
                  __typename
                  id
                }
              } =>
              {
                ... on Category {
                  name
                }
              }
            },
          },
          Flatten(path: "firstProduct.category") {
            Fetch(service: "c") {
              {
                ... on Category {
                  __typename
                  id
                }
              } =>
              {
                ... on Category {
                  name
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
    let query_plan = build_query_plan(
        "fixture/tests/parent-entity-call-complex.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "d") {
          query ($secondProductId:ID!) {
            firstProduct: productFromD(id: "1") {
    ...a        }
            secondProduct: productFromD(id: $secondProductId) {
    ...a        }
          }
          fragment a on Product {
            __typename
            id
            name
          }
        },
        Parallel {
          Flatten(path: "secondProduct") {
            Fetch(service: "a") {
              {
                ... on Product {
                  __typename
                  id
                }
              } =>
              {
                ... on Product {
                  category {
                    details
                  }
                }
              }
            },
          },
          Flatten(path: "secondProduct") {
            Fetch(service: "b") {
              {
                ... on Product {
                  __typename
                  id
                }
              } =>
              {
                ... on Product {
                  category {
                    __typename
                    id
                  }
                }
              }
            },
          },
          Flatten(path: "firstProduct") {
            Fetch(service: "a") {
              {
                ... on Product {
                  __typename
                  id
                }
              } =>
              {
                ... on Product {
                  category {
                    details
                  }
                }
              }
            },
          },
          Flatten(path: "firstProduct") {
            Fetch(service: "b") {
              {
                ... on Product {
                  __typename
                  id
                }
              } =>
              {
                ... on Product {
                  category {
                    __typename
                    id
                  }
                }
              }
            },
          },
        },
        Parallel {
          Flatten(path: "secondProduct.category") {
            Fetch(service: "c") {
              {
                ... on Category {
                  __typename
                  id
                }
              } =>
              {
                ... on Category {
                  name
                }
              }
            },
          },
          Flatten(path: "firstProduct.category") {
            Fetch(service: "c") {
              {
                ... on Category {
                  __typename
                  id
                }
              } =>
              {
                ... on Category {
                  name
                }
              }
            },
          },
        },
      },
    },
    "#);

    insta::assert_snapshot!(format!("{}", serde_json::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
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
                "kind": "Flatten",
                "path": [
                  {
                    "Field": "secondProduct"
                  }
                ],
                "node": {
                  "kind": "Fetch",
                  "serviceName": "a",
                  "operationKind": "query",
                  "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Product{category{details}}}}",
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
              },
              {
                "kind": "Flatten",
                "path": [
                  {
                    "Field": "secondProduct"
                  }
                ],
                "node": {
                  "kind": "Fetch",
                  "serviceName": "b",
                  "operationKind": "query",
                  "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Product{category{__typename id}}}}",
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
              },
              {
                "kind": "Flatten",
                "path": [
                  {
                    "Field": "firstProduct"
                  }
                ],
                "node": {
                  "kind": "Fetch",
                  "serviceName": "a",
                  "operationKind": "query",
                  "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Product{category{details}}}}",
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
              },
              {
                "kind": "Flatten",
                "path": [
                  {
                    "Field": "firstProduct"
                  }
                ],
                "node": {
                  "kind": "Fetch",
                  "serviceName": "b",
                  "operationKind": "query",
                  "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Product{category{__typename id}}}}",
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
              }
            ]
          },
          {
            "kind": "Parallel",
            "nodes": [
              {
                "kind": "Flatten",
                "path": [
                  {
                    "Field": "secondProduct"
                  },
                  {
                    "Field": "category"
                  }
                ],
                "node": {
                  "kind": "Fetch",
                  "serviceName": "c",
                  "operationKind": "query",
                  "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Category{name}}}",
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
              },
              {
                "kind": "Flatten",
                "path": [
                  {
                    "Field": "firstProduct"
                  },
                  {
                    "Field": "category"
                  }
                ],
                "node": {
                  "kind": "Fetch",
                  "serviceName": "c",
                  "operationKind": "query",
                  "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Category{name}}}",
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
              }
            ]
          }
        ]
      }
    }
    "#);

    Ok(())
}
