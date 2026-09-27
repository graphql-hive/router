use crate::query_planner::{
    tests::testkit::{build_query_plan_with_defaults, init_logger},
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
    let query_plan = build_query_plan_with_defaults(
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

// https://github.com/graphql-hive/router/issues/1455
#[test]
fn provides_fieldset_with_typename_and_requires() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          review {
            author {
              username
              __typename
            }
          }
        }"#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/provides-requires-typename.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "reviews") {
          {
            review {
              __typename
              id
            }
          }
        },
        Flatten(path: "review") {
          Fetch(service: "secrets") {
            {
              ... on Review {
                __typename
                id
              }
            } =>
            {
              ... on Review {
                secret
              }
            }
          },
        },
        Flatten(path: "review") {
          Fetch(service: "reviews") {
            {
              ... on Review {
                __typename
                secret
                id
              }
            } =>
            {
              ... on Review {
                author {
                  username
                  __typename
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

/// `label @requires(fields: "name")` on a `User` nested in the entity call for another `User`.
/// The keys for the `names` hop go into that entity call at `other`, not into its parent.
#[test]
fn requires_on_same_type_nested_in_entity_call() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        {
          user {
            other {
              label
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/requires-nested-same-type.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "users") {
          {
            user {
              __typename
              id
            }
          }
        },
        Flatten(path: "user") {
          Fetch(service: "social") {
            {
              ... on User {
                __typename
                id
              }
            } =>
            {
              ... on User {
                other {
                  __typename
                  id
                }
              }
            }
          },
        },
        Flatten(path: "user.other") {
          Fetch(service: "names") {
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
        Flatten(path: "user.other") {
          Fetch(service: "social") {
            {
              ... on User {
                __typename
                name
                id
              }
            } =>
            {
              ... on User {
                label
              }
            }
          },
        },
      },
    },
    "#);

    Ok(())
}

/// Same as above, with a `@provides` field on the way. `other` is a plain `User`, so `name`
/// still comes from `names`.
#[test]
fn requires_on_same_type_nested_under_provides() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        {
          user {
            related {
              other {
                label
              }
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/requires-nested-same-type.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "users") {
          {
            user {
              __typename
              id
            }
          }
        },
        Flatten(path: "user") {
          Fetch(service: "social") {
            {
              ... on User {
                __typename
                id
              }
            } =>
            {
              ... on User {
                related {
                  other {
                    __typename
                    id
                  }
                }
              }
            }
          },
        },
        Flatten(path: "user.related.other") {
          Fetch(service: "names") {
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
        Flatten(path: "user.related.other") {
          Fetch(service: "social") {
            {
              ... on User {
                __typename
                name
                id
              }
            } =>
            {
              ... on User {
                label
              }
            }
          },
        },
      },
    },
    "#);

    Ok(())
}
