use crate::query_planner::{
    tests::testkit::{build_query_plan_with_defaults, init_logger},
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
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/simple-provides.supergraph.graphql",
        document,
    )?;

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
    insta::assert_snapshot!(format!("{}", sonic_rs::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Sequence",
        "nodes": [
          {
            "kind": "Fetch",
            "serviceName": "products",
            "operationKind": "query",
            "operation": "{products{__typename upc}}"
          },
          {
            "kind": "Flatten",
            "path": [
              {
                "Field": "products"
              },
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
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/nested-provides.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "category") {
        {
          products {
            id
            categories {
              name
              id
            }
          }
        }
      },
    },
    "#);
    insta::assert_snapshot!(format!("{}", sonic_rs::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Fetch",
        "serviceName": "category",
        "operationKind": "query",
        "operation": "{products{id categories{name id}}}"
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
    let query_plan = build_query_plan_with_defaults(
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
    insta::assert_snapshot!(format!("{}", sonic_rs::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Fetch",
        "serviceName": "b",
        "operationKind": "query",
        "operation": "{media{__typename ...on Book{id title} ...on Movie{id}}}"
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
    let query_plan = build_query_plan_with_defaults(
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
        Flatten(path: "media.@|[Movie]") {
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
            "operation": "{media{__typename ...on Book{id title} ...on Movie{__typename id}}}"
          },
          {
            "kind": "Flatten",
            "path": [
              {
                "Field": "media"
              },
              "@",
              {
                "TypeCondition": [
                  "Movie"
                ]
              }
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
    let query_plan = build_query_plan_with_defaults(
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
                id
                name
              }
            }
          }
        }
      },
    },
    "#);

    Ok(())
}

// https://github.com/graphql-hive/router/issues/1455
#[test]
fn provides_fieldset_with_typename() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          holder {
            item {
              __typename
              label
            }
          }
        }"#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/provides-typename.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "provider") {
        {
          holder {
            item {
              __typename
              label
            }
          }
        }
      },
    },
    "#);
    Ok(())
}

/// `media` is `@external` in the provider, so its union node only exists for the `@provides`
/// and gets its `__typename` from the plain union node.
#[test]
fn provides_external_union_field_with_typename() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          holder {
            media {
              __typename
              ... on Book {
                title
              }
            }
          }
        }"#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/provides-typename-union.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "provider") {
        {
          holder {
            media {
              __typename
              ... on Book {
                title
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
fn provides_fieldset_with_nested_typename() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          holder {
            item {
              __typename
              nested {
                __typename
                label
              }
            }
          }
        }"#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/provides-typename-nested.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "provider") {
        {
          holder {
            item {
              __typename
              nested {
                __typename
                label
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
fn provides_fieldset_with_typename_on_list() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          holder {
            items {
              __typename
              label
            }
          }
        }"#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/provides-typename-list.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "provider") {
        {
          holder {
            items {
              __typename
              label
            }
          }
        }
      },
    },
    "#);
    Ok(())
}

#[test]
fn provides_fieldset_with_typename_on_interface() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          book {
            animals {
              __typename
              ... on Dog {
                __typename
                name
              }
            }
          }
        }"#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/provides-typename-interface.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "a") {
        {
          book {
            animals {
              __typename
              ... on Dog {
                __typename
                name
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
    let query_plan = build_query_plan_with_defaults(
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
              ... on Book {
                animals {
                  __typename
                  ... on Cat {
                    __typename
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
        Flatten(path: "media|[Book].animals.@|[Cat]") {
          Fetch(service: "c") {
            {
              ... on Cat {
                __typename
                id
              }
            } =>
            {
              ... on Cat {
                age
              }
            }
          },
        },
      },
    },
    "#);

    Ok(())
}

/// `shelf` provides `title` of `Book` (`String`) and `Movie` (`String!`) through `media`. The
/// fixture is the one Apollo composes, minus `shelf`'s `external: true` entry on `Movie.title`,
/// so the planner only has the supergraph's `Movie.title: String` to go on. That type says
/// nothing about what `shelf` declares: the second `title` gets an alias, so the operation stays
/// valid in `shelf`, and the client still gets `title`.
#[test]
fn provided_field_missing_from_the_subgraph_gets_an_alias() -> Result<(), Box<dyn Error>> {
    use graphql_tools::validation::{rules::default_rules_validation_plan, validate::validate};

    init_logger();
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/provides-missing-external-entry.supergraph.graphql",
        parse_operation("{ media { ... on Book { title } ... on Movie { title } } }"),
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "shelf") {
        {
          media {
            __typename
            ... on Book {
              title
            }
            ... on Movie {
              _internal_qp_alias_0: title
            }
          }
        }
      },
    },
    "#);

    // `Movie`'s alias goes back to `title` in the client's response
    let plan = serde_json::to_value(&query_plan)?;
    insta::assert_snapshot!(serde_json::to_string_pretty(&plan["node"]["outputRewrites"])?, @r#"
    [
      {
        "KeyRenamer": {
          "path": [
            {
              "Key": "media"
            },
            {
              "TypenameEquals": [
                "Movie"
              ]
            },
            {
              "Key": "_internal_qp_alias_0"
            }
          ],
          "renameKeyTo": "title"
        }
      }
    ]
    "#);

    let operation = plan["node"]["operation"].as_str().expect("one fetch");
    let shelf = crate::query_planner::utils::parsing::parse_schema(
        r#"
        type Query { media: [Media] }
        union Media = Book | Movie
        type Book { id: ID! title: String }
        type Movie { id: ID! title: String! }
        "#,
    );
    let errors = validate(
        &shelf,
        &parse_operation(operation),
        &default_rules_validation_plan(),
    );
    assert!(errors.is_empty(), "{errors:?}\n{operation}");

    Ok(())
}
