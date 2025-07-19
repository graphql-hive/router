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

    insta::assert_snapshot!(format!("{}", query_plan), @r###"
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
          Flatten(path: "topProducts.@") {
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
                }
              }
            },
          },
          Flatten(path: "users.@") {
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
                    body
                    product {
                      __typename
                      upc
                      reviews {
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
                    }
                  }
                }
              }
            },
          },
        },
        Parallel {
          Flatten(path: "topProducts.@.reviews.@.author.reviews.@.product") {
            Fetch(service: "products") {
              {
                ... on Product {
                  __typename
                  upc
                }
              } =>
              {
                ... on Product {
                  price
                  weight
                  name
                }
              }
            },
          },
          Flatten(path: "topProducts.@.reviews.@.author") {
            Fetch(service: "accounts") {
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
          Flatten(path: "users.@.reviews.@.product") {
            Fetch(service: "products") {
              {
                ... on Product {
                  __typename
                  upc
                }
              } =>
              {
                ... on Product {
                  price
                  weight
                  name
                }
              }
            },
          },
          Flatten(path: "users.@.reviews.@.product.reviews.@.author.reviews.@.product") {
            Fetch(service: "products") {
              {
                ... on Product {
                  __typename
                  upc
                }
              } =>
              {
                ... on Product {
                  price
                  weight
                  name
                }
              }
            },
          },
          Flatten(path: "users.@.reviews.@.product.reviews.@.author") {
            Fetch(service: "accounts") {
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
        },
        Parallel {
          Flatten(path: "topProducts.@.reviews.@.author.reviews.@.product") {
            Fetch(service: "inventory") {
              {
                ... on Product {
                  __typename
                  upc
                  price
                  weight
                }
              } =>
              {
                ... on Product {
                  inStock
                  shippingEstimate
                }
              }
            },
          },
          Flatten(path: "users.@.reviews.@.product") {
            Fetch(service: "inventory") {
              {
                ... on Product {
                  __typename
                  upc
                  price
                  weight
                }
              } =>
              {
                ... on Product {
                  inStock
                  shippingEstimate
                }
              }
            },
          },
          Flatten(path: "users.@.reviews.@.product.reviews.@.author.reviews.@.product") {
            Fetch(service: "inventory") {
              {
                ... on Product {
                  __typename
                  upc
                  price
                  weight
                }
              } =>
              {
                ... on Product {
                  inStock
                  shippingEstimate
                }
              }
            },
          },
        },
      },
    },
    "###);

    Ok(())
}
