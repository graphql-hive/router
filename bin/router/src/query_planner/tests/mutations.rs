use crate::query_planner::{
    tests::testkit::{build_query_plan_with_defaults, init_logger},
    utils::parsing::parse_operation,
};
use std::error::Error;

// TODO: try to reproduce shared_root for mutations

#[test]
fn mutations() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        mutation {
          addProduct(input: { name: "new", price: 599.99 }) {
            name
            price
            isExpensive
            isAvailable
          }
        }
        "#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/tests/mutations.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          mutation {
            addProduct(input: {name: "new", price: 599.99}) {
              __typename
              name
              price
              id
            }
          }
        },
        Flatten(path: "addProduct") {
          Fetch(service: "b") {
            {
              ... on Product {
                __typename
                price
                id
              }
            } =>
            {
              ... on Product {
                isExpensive
                isAvailable
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
            "serviceName": "a",
            "operationKind": "mutation",
            "operation": "mutation{addProduct(input: {name: \"new\", price: 599.99}){__typename name price id}}"
          },
          {
            "kind": "Flatten",
            "path": [
              {
                "Field": "addProduct"
              }
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "b",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Product{isExpensive isAvailable}}}",
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
fn many_fields_two_same_graph() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        mutation {
          five: add(num: 5)
          ten: multiply(by: 2)
          twelve: add(num: 2)
          final: delete
        }
        "#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/tests/mutations.supergraph.graphql", document)?;
    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "c") {
          mutation {
            five: add(num: 5)
          }
        },
        Fetch(service: "a") {
          mutation {
            ten: multiply(by: 2)
          }
        },
        Fetch(service: "c") {
          mutation {
            twelve: add(num: 2)
          }
        },
        Fetch(service: "b") {
          mutation {
            final: delete
          }
        },
      },
    },
    "#);

    let document = parse_operation(
        r#"
        mutation {
          five: add(num: 5)
          seven: add(num: 2)
          fourteen: multiply(by: 2)
          sixteen: add(num: 2)
          final: delete
        }
        "#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/tests/mutations.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "c") {
          mutation {
            five: add(num: 5)
            seven: add(num: 2)
          }
        },
        Fetch(service: "a") {
          mutation {
            fourteen: multiply(by: 2)
          }
        },
        Fetch(service: "c") {
          mutation {
            sixteen: add(num: 2)
          }
        },
        Fetch(service: "b") {
          mutation {
            final: delete
          }
        },
      },
    },
    "#);

    Ok(())
}

/// Three mutations in a row on one subgraph go in one fetch. After the first two are merged,
/// the third one has to still count as next to them.
#[test]
fn three_fields_in_a_row_same_graph() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        mutation {
          one: add(num: 1)
          two: add(num: 2)
          three: add(num: 3)
          final: delete
        }
        "#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/tests/mutations.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "c") {
          mutation {
            one: add(num: 1)
            two: add(num: 2)
            three: add(num: 3)
          }
        },
        Fetch(service: "b") {
          mutation {
            final: delete
          }
        },
      },
    },
    "#);

    Ok(())
}

/// `count` starts only once `create` is done, and that includes `isExpensive` from `b`.
#[test]
fn next_field_waits_for_entity_fetch_of_previous_one() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        mutation {
          create: addProduct(input: { name: "new", price: 599.99 }) {
            isExpensive
          }
          count: add(num: 1)
        }
        "#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/tests/mutations.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          mutation {
            create: addProduct(input: {name: "new", price: 599.99}) {
              __typename
              id
              price
            }
          }
        },
        Flatten(path: "create") {
          Fetch(service: "b") {
            {
              ... on Product {
                __typename
                price
                id
              }
            } =>
            {
              ... on Product {
                isExpensive
              }
            }
          },
        },
        Fetch(service: "c") {
          mutation {
            count: add(num: 1)
          }
        },
      },
    },
    "#);

    Ok(())
}

/// Same as above, with the entity fetch behind `@include`.
#[test]
fn next_field_waits_for_conditional_entity_fetch_of_previous_one() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        mutation ($includeRemote: Boolean!) {
          create: addProduct(input: { name: "new", price: 599.99 }) {
            ... on Product @include(if: $includeRemote) {
              isExpensive
            }
          }
          count: add(num: 1)
        }
        "#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/tests/mutations.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          mutation ($includeRemote:Boolean!) {
            create: addProduct(input: {name: "new", price: 599.99}) {
              ... on Product @include(if: $includeRemote) {
                __typename
                id
                price
              }
            }
          }
        },
        Include(if: $includeRemote) {
          Flatten(path: "create|[Product]") {
            Fetch(service: "b") {
              {
                ... on Product {
                  __typename
                  price
                  id
                }
              } =>
              {
                ... on Product {
                  isExpensive
                }
              }
            },
          },
        },
        Fetch(service: "c") {
          mutation {
            count: add(num: 1)
          }
        },
      },
    },
    "#);

    Ok(())
}

/// `create` and `multiply` are both in `a`, so they go in one request, like in Apollo, even though
/// `create` needs `b` for `isExpensive`. `b` runs after that request, once the product exists.
#[test]
fn neighbours_in_one_graph_share_a_fetch_when_first_needs_another_fetch(
) -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        mutation {
          create: addProduct(input: { name: "new", price: 599.99 }) {
            isExpensive
          }
          double: multiply(by: 2)
        }
        "#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/tests/mutations.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          mutation {
            create: addProduct(input: {name: "new", price: 599.99}) {
              __typename
              id
              price
            }
            double: multiply(by: 2)
          }
        },
        Flatten(path: "create") {
          Fetch(service: "b") {
            {
              ... on Product {
                __typename
                price
                id
              }
            } =>
            {
              ... on Product {
                isExpensive
              }
            }
          },
        },
      },
    },
    "#);

    Ok(())
}

/// `create` and `double` go to `a` in one request. `count` is in `c`, so it waits for the whole
/// group, `isExpensive` from `b` included.
#[test]
fn next_graph_waits_for_entity_fetch_of_the_group() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        mutation {
          create: addProduct(input: { name: "new", price: 599.99 }) {
            isExpensive
          }
          double: multiply(by: 2)
          count: add(num: 1)
        }
        "#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/tests/mutations.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          mutation {
            create: addProduct(input: {name: "new", price: 599.99}) {
              __typename
              id
              price
            }
            double: multiply(by: 2)
          }
        },
        Flatten(path: "create") {
          Fetch(service: "b") {
            {
              ... on Product {
                __typename
                price
                id
              }
            } =>
            {
              ... on Product {
                isExpensive
              }
            }
          },
        },
        Fetch(service: "c") {
          mutation {
            count: add(num: 1)
          }
        },
      },
    },
    "#);

    Ok(())
}

/// Same as above, with the entity fetch on the second field of the group.
#[test]
fn next_graph_waits_for_entity_fetch_of_a_later_field_in_the_group() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        mutation {
          double: multiply(by: 2)
          create: addProduct(input: { name: "new", price: 599.99 }) {
            isExpensive
          }
          count: add(num: 1)
        }
        "#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/tests/mutations.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          mutation {
            double: multiply(by: 2)
            create: addProduct(input: {name: "new", price: 599.99}) {
              __typename
              id
              price
            }
          }
        },
        Flatten(path: "create") {
          Fetch(service: "b") {
            {
              ... on Product {
                __typename
                price
                id
              }
            } =>
            {
              ... on Product {
                isExpensive
              }
            }
          },
        },
        Fetch(service: "c") {
          mutation {
            count: add(num: 1)
          }
        },
      },
    },
    "#);

    Ok(())
}

/// `final` is in `b`, like the entity fetch for `isExpensive`. It's a mutation of its own, so it
/// still waits for that fetch, and the two stay apart.
#[test]
fn next_graph_waits_for_entity_fetch_to_the_same_graph() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        mutation {
          create: addProduct(input: { name: "new", price: 599.99 }) {
            isExpensive
          }
          double: multiply(by: 2)
          final: delete
        }
        "#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/tests/mutations.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          mutation {
            create: addProduct(input: {name: "new", price: 599.99}) {
              __typename
              id
              price
            }
            double: multiply(by: 2)
          }
        },
        Flatten(path: "create") {
          Fetch(service: "b") {
            {
              ... on Product {
                __typename
                price
                id
              }
            } =>
            {
              ... on Product {
                isExpensive
              }
            }
          },
        },
        Fetch(service: "b") {
          mutation {
            final: delete
          }
        },
      },
    },
    "#);

    Ok(())
}

/// `one` and `two` are both in `c`, but `create` is between them, so they aren't a group.
#[test]
fn fields_of_one_graph_with_another_graph_between_stay_apart() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        mutation {
          one: add(num: 1)
          create: addProduct(input: { name: "new", price: 599.99 }) {
            isExpensive
          }
          two: add(num: 2)
        }
        "#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/tests/mutations.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "c") {
          mutation {
            one: add(num: 1)
          }
        },
        Fetch(service: "a") {
          mutation {
            create: addProduct(input: {name: "new", price: 599.99}) {
              __typename
              id
              price
            }
          }
        },
        Flatten(path: "create") {
          Fetch(service: "b") {
            {
              ... on Product {
                __typename
                price
                id
              }
            } =>
            {
              ... on Product {
                isExpensive
              }
            }
          },
        },
        Fetch(service: "c") {
          mutation {
            two: add(num: 2)
          }
        },
      },
    },
    "#);

    Ok(())
}

/// A field under `@include` still goes in the request of its group.
#[test]
fn conditional_field_stays_in_its_group() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        mutation ($x: Boolean!) {
          one: add(num: 1) @include(if: $x)
          two: add(num: 2)
        }
        "#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/tests/mutations.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "c") {
        mutation ($x:Boolean!) {
          one: add(num: 1) @include(if: $x)
          two: add(num: 2)
        }
      },
    },
    "#);

    Ok(())
}
