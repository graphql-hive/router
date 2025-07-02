use crate::{
    tests::testkit::{build_query_plan, init_logger},
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
    let query_plan = build_query_plan("fixture/tests/mutations.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          mutation {
            addProduct(input: {name: "new", price: 599.99}) {
              __typename
              id
              name
              price
            }
          }
        },
        Flatten(path: "addProduct") {
          Fetch(service: "b") {
              ... on Product {
                __typename
                price
                id
              }
            } =>
            {
              ... on Product {
                isAvailable
                isExpensive
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
            "operationKind": "mutation",
            "operation": "mutation{addProduct(input: {name: \"new\", price: 599.99}){__typename id name price}}"
          },
          {
            "kind": "Flatten",
            "path": [
              "addProduct"
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "b",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Product{isAvailable isExpensive}}}",
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
    let query_plan = build_query_plan("fixture/tests/mutations.supergraph.graphql", document)?;
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
    let query_plan = build_query_plan("fixture/tests/mutations.supergraph.graphql", document)?;

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
