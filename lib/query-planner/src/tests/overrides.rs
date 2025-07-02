use crate::{
    tests::testkit::{build_query_plan, init_logger},
    utils::parsing::parse_operation,
};
use std::error::Error;

#[test]
fn single_simple_overrides() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          feed {
            createdAt
          }
        }"#,
    );
    let query_plan = build_query_plan(
        "fixture/tests/simple_overrides.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "b") {
        {
          feed {
            createdAt
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
        "operation": "query{feed{createdAt}}"
      }
    }
    "#);
    Ok(())
}

#[test]
fn two_fields_simple_overrides() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          aFeed {
            createdAt
          }
          bFeed {
            createdAt
          }
        }"#,
    );
    let query_plan = build_query_plan(
        "fixture/tests/simple_overrides.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Parallel {
          Fetch(service: "b") {
            {
              bFeed {
                createdAt
              }
            }
          },
          Fetch(service: "a") {
            {
              aFeed {
                __typename
                id
              }
            }
          },
        },
        Flatten(path: "aFeed.@") {
          Fetch(service: "b") {
              ... on Post {
                __typename
                id
              }
            } =>
            {
              ... on Post {
                createdAt
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
            "kind": "Parallel",
            "nodes": [
              {
                "kind": "Fetch",
                "serviceName": "b",
                "operationKind": "query",
                "operation": "query{bFeed{createdAt}}"
              },
              {
                "kind": "Fetch",
                "serviceName": "a",
                "operationKind": "query",
                "operation": "query{aFeed{__typename id}}"
              }
            ]
          },
          {
            "kind": "Flatten",
            "path": [
              "aFeed",
              "@"
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "b",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Post{createdAt}}}",
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
          }
        ]
      }
    }
    "#);
    Ok(())
}

#[test]
fn override_object_field_but_interface_is_requested() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          feed {
            id
            createdAt
          }
        }
        "#,
    );
    let query_plan = build_query_plan(
        "fixture/tests/override-type-interface.supergraph.graphql",
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
              ... on ImagePost {
                __typename
                id
              }
            }
          }
        },
        Flatten(path: "feed.@") {
          Fetch(service: "b") {
              ... on ImagePost {
                __typename
                id
              }
            } =>
            {
              ... on ImagePost {
                createdAt
              }
            }
          },
        },
      },
    },
    "#);
    Ok(())
}
