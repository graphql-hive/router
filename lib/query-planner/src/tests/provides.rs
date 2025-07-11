use crate::{
    tests::testkit::{build_query_plan, init_logger},
    utils::parsing::parse_operation,
};
use std::error::Error;

#[test]
fn simple_provides() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          products {
            reviews {
              author {
                username
              }
            }
          }
        }"#,
    );
    let query_plan =
        build_query_plan("fixture/tests/simple-provides.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "products") {
          {
            products {
              __typename
              upc
            }
          }
        },
        Flatten(path: "products.@") {
          Fetch(service: "reviews") {
            {
              ... on Product {
                __typename
                upc
              }
            } =>
            {
              ... on Product {
                reviews {
                  author {
                    username
                  }
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
            "serviceName": "products",
            "operationKind": "query",
            "operation": "query{products{__typename upc}}"
          },
          {
            "kind": "Flatten",
            "path": [
              "products",
              "@"
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "reviews",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Product{reviews{author{username}}}}}",
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
                      "name": "upc"
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
fn nested_provides() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          products {
            id
            categories {
              id
              name
            }
          }
        }"#,
    );
    let query_plan =
        build_query_plan("fixture/tests/nested-provides.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "category") {
        {
          products {
            categories {
              id
              name
            }
            id
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
        "serviceName": "category",
        "operationKind": "query",
        "operation": "query{products{categories{id name} id}}"
      }
    }
    "#);

    Ok(())
}

#[test]
fn provides_on_union() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          media {
            ... on Book {
              id
              title
            }
            ... on Movie {
              id
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan(
        "fixture/tests/provides-on-union.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "b") {
        {
          media {
            __typename
            ... on Book {
              id
              title
            }
            ... on Movie {
              id
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
        "serviceName": "b",
        "operationKind": "query",
        "operation": "query{media{__typename ...on Book{id title} ...on Movie{id}}}"
      }
    }
    "#);

    let document = parse_operation(
        r#"
        query {
          media {
            ... on Book {
              id
              title
            }
            ... on Movie {
              id
              title
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan(
        "fixture/tests/provides-on-union.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "b") {
          {
            media {
              __typename
              ... on Book {
                id
                title
              }
              ... on Movie {
                __typename
                id
              }
            }
          }
        },
        Flatten(path: "media") {
          Fetch(service: "c") {
            {
              ... on Movie {
                __typename
                id
              }
            } =>
            {
              ... on Movie {
                title
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
            "serviceName": "b",
            "operationKind": "query",
            "operation": "query{media{__typename ...on Book{id title} ...on Movie{__typename id}}}"
          },
          {
            "kind": "Flatten",
            "path": [
              "media"
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "c",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Movie{title}}}",
              "requires": [
                {
                  "kind": "InlineFragment",
                  "typeCondition": "Movie",
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
fn provides_on_interface_1_test() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
          query {
            media {
              id
              animals {
                id
                name
              }
            }
          }
        "#,
    );
    let query_plan = build_query_plan(
        "fixture/tests/provides-on-interface.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "b") {
        {
          media {
            __typename
            id
            ... on Book {
              animals {
                __typename
                ... on Cat {
                  id
                  name
                }
                ... on Dog {
                  id
                  name
                }
              }
            }
          }
        }
      },
    },
    "#);

    Ok(())
}

#[test]
fn provides_on_interface_2_test() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
          query {
            media {
              id
              animals {
                id
                name
                ... on Cat {
                  age
                }
              }
            }
          }
        "#,
    );
    let query_plan = build_query_plan(
        "fixture/tests/provides-on-interface.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
      QueryPlan {
        Sequence {
          Fetch(service: "b") {
            {
              media {
                __typename
                id
                animals {
                  __typename
                  id
                  name
                }
                ... on Book {
                  __typename
                  id
                }
              }
            }
          },
          Flatten(path: "media") {
            Fetch(service: "c") {
              {
                ... on Book {
                  __typename
                  id
                }
              } =>
              {
                ... on Book {
                  animals {
                    __typename
                    ... on Cat {
                      age
                    }
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
