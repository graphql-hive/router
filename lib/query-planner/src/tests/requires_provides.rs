use crate::{
    tests::testkit::{build_query_plan, init_logger},
    utils::parsing::parse_operation,
};
use std::error::Error;

#[test]
fn simple_requires_provides() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          me {
            reviews {
              id
              author {
                id
                username
              }
              product {
                inStock
              }
            }
          }
        }"#,
    );
    let query_plan = build_query_plan(
        "fixture/tests/requires-provides.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "accounts") {
          {
            me {
              __typename
              id
            }
          }
        },
        Flatten(path: "me") {
          Fetch(service: "reviews") {
            {
              ... on User {
                __typename
                id
              }
            } =>
            {
              ... on User {
                reviews {
                  id
                  author {
                    id
                    username
                  }
                  product {
                    __typename
                    upc
                  }
                }
              }
            }
          },
        },
        Flatten(path: "me.reviews.@.product") {
          Fetch(service: "inventory") {
            {
              ... on Product {
                __typename
                upc
              }
            } =>
            {
              ... on Product {
                inStock
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
            "kind": "Fetch",
            "id": 2,
            "serviceName": "accounts",
            "operationKind": "query",
            "operation": "{me{__typename id}}"
          },
          {
            "kind": "Flatten",
            "path": [
              {
                "Field": "me"
              }
            ],
            "node": {
              "kind": "Fetch",
              "id": 3,
              "serviceName": "reviews",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on User{reviews{id author{id username} product{__typename upc}}}}}",
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
          },
          {
            "kind": "Flatten",
            "path": [
              {
                "Field": "me"
              },
              {
                "Field": "reviews"
              },
              "@",
              {
                "Field": "product"
              }
            ],
            "node": {
              "kind": "Fetch",
              "id": 4,
              "serviceName": "inventory",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Product{inStock}}}",
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
