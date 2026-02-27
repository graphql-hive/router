mod alias;
mod arguments;
mod fragments;
mod include_skip;
mod interface;
mod interface_object;
mod interface_object_with_requires;
mod issues;
mod mutations;
mod object_entities;
mod override_requires;
mod overrides;
mod provides;
mod requires;
mod requires_circular;
mod requires_fragments;
mod requires_provides;
mod requires_requires;
mod root_types;
mod testkit;
mod union;

use crate::{
    tests::testkit::{build_query_plan, init_logger},
    utils::parsing::parse_operation,
};

#[test]
fn test_bench_operation() -> Result<(), Box<dyn std::error::Error>> {
    init_logger();
    let document = parse_operation(
        &std::fs::read_to_string("../../bench/operation.graphql")
            .expect("Unable to read input file"),
    );
    let query_plan = build_query_plan("../../bench/supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Parallel {
          Fetch(service: "products") {
            {
              topProducts {
                __typename
                upc
                name
                price
                weight
              }
            }
          },
          Fetch(service: "accounts") {
            {
              users {
                __typename
                id
                username
                name
              }
            }
          },
        },
        Parallel {
          Flatten(path: "topProducts.@") {
            Fetch(service: "inventory") {
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
                  inStock
                }
              }
            },
          },
          BatchFetch(service: "reviews") {
            entityBatch(originalFetchCount: 2) {
              _e0 {
                paths: [
                  "topProducts.@"
                ]
                {
                  ... on Product {
                    __typename
                    upc
                  }
                }
              }
              _e1 {
                paths: [
                  "users.@"
                ]
                {
                  ... on User {
                    __typename
                    id
                  }
                }
              }
            }
            {
              _e0: _entities(representations: $__batch_reps_0) {
                ... on Product {
                  reviews {
                    ...a
                  }
                }
              }
              _e1: _entities(representations: $__batch_reps_1) {
                ... on User {
                  reviews {
                    id
                    body
                    product {
                      __typename
                      upc
                      reviews {
                        ...a
                      }
                    }
                  }
                }
              }
            }
            fragment a on Review {
              id
              body
              author {
                __typename
                id
                reviews {
                  id
                  body
                  product {
                    __typename
                    upc
                  }
                }
                username
              }
            }
          },
        },
        Parallel {
          BatchFetch(service: "products") {
            entityBatch(originalFetchCount: 3) {
              _e0 {
                paths: [
                  "topProducts.@.reviews.@.author.reviews.@.product"
                  "users.@.reviews.@.product"
                  "users.@.reviews.@.product.reviews.@.author.reviews.@.product"
                ]
                {
                  ... on Product {
                    __typename
                    upc
                  }
                }
              }
            }
            {
              _e0: _entities(representations: $__batch_reps_0) {
                ... on Product {
                  price
                  weight
                  name
                }
              }
            }
          },
          BatchFetch(service: "accounts") {
            entityBatch(originalFetchCount: 2) {
              _e0 {
                paths: [
                  "topProducts.@.reviews.@.author"
                  "users.@.reviews.@.product.reviews.@.author"
                ]
                {
                  ... on User {
                    __typename
                    id
                  }
                }
              }
            }
            {
              _e0: _entities(representations: $__batch_reps_0) {
                ... on User {
                  name
                }
              }
            }
          },
        },
        BatchFetch(service: "inventory") {
          entityBatch(originalFetchCount: 3) {
            _e0 {
              paths: [
                "topProducts.@.reviews.@.author.reviews.@.product"
                "users.@.reviews.@.product"
                "users.@.reviews.@.product.reviews.@.author.reviews.@.product"
              ]
              {
                ... on Product {
                  __typename
                  upc
                  price
                  weight
                }
              }
            }
          }
          {
            _e0: _entities(representations: $__batch_reps_0) {
              ... on Product {
                inStock
                shippingEstimate
              }
            }
          }
        },
      },
    },
    "#);

    Ok(())
}
