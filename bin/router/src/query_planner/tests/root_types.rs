use crate::query_planner::{
    tests::testkit::{build_query_plan_from_sdl, build_query_plan_with_defaults, init_logger},
    utils::parsing::parse_operation,
};
use std::error::Error;

#[test]
fn shared_root() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          product {
            id
            name {
              id
              brand
              model
            }
            category {
              id
              name
            }
            price {
              id
              amount
              currency
            }
          }
        }"#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/tests/shared-root.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Parallel {
        Fetch(service: "price") {
          {
            product {
              price {
                id
                amount
                currency
              }
            }
          }
        },
        Fetch(service: "category") {
          {
            product {
              category {
                id
                name
              }
              id
            }
          }
        },
        Fetch(service: "name") {
          {
            product {
              name {
                id
                brand
                model
              }
            }
          }
        },
      },
    },
    "#);
    insta::assert_snapshot!(format!("{}", sonic_rs::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Parallel",
        "nodes": [
          {
            "kind": "Fetch",
            "serviceName": "price",
            "operationKind": "query",
            "operation": "{product{price{id amount currency}}}"
          },
          {
            "kind": "Fetch",
            "serviceName": "category",
            "operationKind": "query",
            "operation": "{product{category{id name} id}}"
          },
          {
            "kind": "Fetch",
            "serviceName": "name",
            "operationKind": "query",
            "operation": "{product{name{id brand model}}}"
          }
        ]
      }
    }
    "#);
    Ok(())
}

#[test]
fn shareable_subscription_field_uses_one_stream() -> Result<(), Box<dyn Error>> {
    init_logger();
    // `orderUpdated` lives in both subgraphs, `total` only in a and `eta` only in b.
    // Two streams can't be put together, so one subgraph streams
    // and the rest comes through an entity call on each event.
    // Composition rejects a subscription field in more than one subgraph, @shareable or not,
    // so Hive never hands us this schema. The planner should still stream from one subgraph.
    let supergraph = r#"
        enum join__Graph {
            A @join__graph(name: "a", url: "")
            B @join__graph(name: "b", url: "")
        }
        type Query @join__type(graph: A) @join__type(graph: B) {
            ping: Boolean @join__field(graph: A)
        }
        type Subscription @join__type(graph: A) @join__type(graph: B) {
            orderUpdated: Order
        }
        type Order @join__type(graph: A, key: "id") @join__type(graph: B, key: "id") {
            id: ID!
            total: Int @join__field(graph: A)
            eta: String @join__field(graph: B)
        }
    "#;
    let query_plan = build_query_plan_from_sdl(
        supergraph,
        parse_operation("subscription { orderUpdated { total eta } }"),
        Default::default(),
        Default::default(),
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Subscription {
          Fetch(service: "a") {
            subscription {
              orderUpdated {
                __typename
                total
                id
              }
            }
          },
        },
        Flatten(path: "orderUpdated") {
          Fetch(service: "b") {
            {
              ... on Order {
                __typename
                id
              }
            } =>
            {
              ... on Order {
                eta
              }
            }
          },
        },
      },
    },
    "#);

    Ok(())
}
