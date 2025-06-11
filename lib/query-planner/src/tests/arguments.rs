use crate::{
    tests::testkit::{build_query_plan, init_logger},
    utils::parsing::parse_operation,
};
use std::error::Error;

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
            tracks(limit: 5, offset: 10) {
              edges {
                node {
                  name
                }
              }
            }
            genres
            name
            albumType
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
        "operation": "{album(id: \"5\"){tracks(limit: 5, offset: 10){edges{node{name}}} genres name albumType}}"
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
        {
          album(id: $id) {
            tracks(limit: $limit, offset: 10) {
              edges {
                node {
                  name
                }
              }
            }
            genres
            name
            albumType
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
        "operation": "{album(id: $id){tracks(limit: $limit, offset: 10){edges{node{name}}} genres name albumType}}"
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
            secondProduct: productFromD(id: "2") {
              __typename
              id
              name
            }
            firstProduct: productFromD(id: "1") {
              __typename
              id
              name
            }
          }
        },
        Parallel {
          Flatten(path: "firstProduct") {
            Fetch(service: "b") {
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
          Flatten(path: "secondProduct") {
            Fetch(service: "a") {
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
        },
        Parallel {
          Flatten(path: "firstProduct.category") {
            Fetch(service: "c") {
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
          Flatten(path: "secondProduct.category") {
            Fetch(service: "c") {
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
          {
            secondProduct: productFromD(id: $secondProductId) {
              __typename
              id
              name
            }
            firstProduct: productFromD(id: "1") {
              __typename
              id
              name
            }
          }
        },
        Parallel {
          Flatten(path: "firstProduct") {
            Fetch(service: "b") {
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
          Flatten(path: "secondProduct") {
            Fetch(service: "a") {
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
        },
        Parallel {
          Flatten(path: "firstProduct.category") {
            Fetch(service: "c") {
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
          Flatten(path: "secondProduct.category") {
            Fetch(service: "c") {
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
            "operation": "{secondProduct: productFromD(id: $secondProductId){__typename id name} firstProduct: productFromD(id: \"1\"){__typename id name}}"
          },
          {
            "kind": "Parallel",
            "nodes": [
              {
                "kind": "Flatten",
                "path": [
                  "firstProduct"
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
                  "firstProduct"
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
                  "secondProduct"
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
                  "secondProduct"
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
              }
            ]
          },
          {
            "kind": "Parallel",
            "nodes": [
              {
                "kind": "Flatten",
                "path": [
                  "firstProduct",
                  "category"
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
                  "secondProduct",
                  "category"
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
