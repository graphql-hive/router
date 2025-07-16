use crate::ast::document::Document;
use crate::ast::minification::stats::Stats;
use crate::ast::minification::transform::transform_operation;
use crate::ast::{minification::error::MinificationError, operation::OperationDefinition};
use crate::state::supergraph_state::{OperationKind, SupergraphState};

pub mod error;
mod selection_id;
mod stats;
mod transform;

pub fn minify_operation(
    operation: OperationDefinition,
    supergraph: &SupergraphState,
) -> Result<Document, MinificationError> {
    let root_type_name = get_root_type_name(&operation, supergraph)?.to_string();
    let stats = Stats::from_operation(&operation.selection_set, supergraph, &root_type_name)?;
    transform_operation(supergraph, stats, &root_type_name, operation)
}

fn get_root_type_name<'a>(
    operation: &'a OperationDefinition,
    supergraph: &'a SupergraphState,
) -> Result<&'a str, MinificationError> {
    Ok(match operation.operation_kind {
        None => &supergraph.query_type,
        Some(OperationKind::Query) => &supergraph.query_type,
        Some(OperationKind::Mutation) => supergraph
            .mutation_type
            .as_ref()
            .ok_or_else(|| MinificationError::TypeNotFound("Mutation".to_string()))?,
        Some(OperationKind::Subscription) => supergraph
            .subscription_type
            .as_ref()
            .ok_or_else(|| MinificationError::TypeNotFound("Subscription".to_string()))?,
    })
}

#[cfg(test)]
mod tests {
    use graphql_parser::parse_query;
    use graphql_parser::query::Document;

    use crate::ast::minification::minify_operation;
    use crate::ast::normalization::normalize_operation;
    use crate::ast::operation::OperationDefinition;
    use crate::state::supergraph_state::SupergraphState;
    use crate::utils::parsing::parse_schema;

    fn pretty_query(query_str: String) -> String {
        format!(
            "{}",
            parse_query::<&str>(&query_str)
                .unwrap_or_else(|_| panic!("failed to parse: {}", query_str))
        )
    }

    #[test]
    fn minification_test() {
        let schema = parse_schema(
            r#"
            interface Product {
              id: ID!
              name: String
              distributor: BusinessEntity
            }

            interface BusinessEntity {
              id: ID!
              name: String
            }

            type Query {
              product(id: ID!): Product
            }

            type Book implements Product {
              id: ID!
              name: String
              distributor: BusinessEntity
              relatedProducts: [Product]
            }

            type Electronic implements Product {
              id: ID!
              name: String
              distributor: BusinessEntity
            }

            type Supplier implements BusinessEntity {
              id: ID!
              name: String
              licenseNumber: String
            }

            type Vendor implements BusinessEntity {
              id: ID!
              name: String
              preferred: Boolean
            }

            type Manufacturer implements BusinessEntity {
                id: ID!
                name: String
                country: String
            }
        "#,
        );

        let supergraph = SupergraphState::new(&schema);

        let document: Document<'static, String> = parse_query(
            r#"
        query ($id: ID!) {
          product(id: $id) {
            ...ProductFields
          }
        }

        fragment ProductFields on Product {
          id
          name
          distributor {
            ...DistributorFull
          }
          ... on Book {
            relatedProducts {
              id
              name
              distributor {
                ...DistributorFull
              }
              ... on Book {
                relatedProducts {
                  id
                  distributor {
                    ...DistributorPartial
                  }
                }
              }
            }
          }
          ... on Electronic {
            distributor {
              ...DistributorPartial
            }
          }
        }

        fragment DistributorFull on BusinessEntity {
          id
          name
          ... on Supplier {
            licenseNumber
            __typename
          }
          ... on Vendor {
            preferred
            __typename
          }
          ... on Manufacturer {
            country
            __typename
          }
        }

        fragment DistributorPartial on BusinessEntity {
          id
          name
          ... on Manufacturer {
            country
            __typename
          }
        }
        "#,
        )
        .expect("Failed to parse query");

        let normalized =
            normalize_operation(&supergraph, &document, None).expect("Failed to normalized");
        let operation: OperationDefinition = normalized.operation;

        // After normalization
        insta::assert_snapshot!(
            pretty_query(operation.to_string()),
            @r"
        query($id: ID!) {
          product(id: $id) {
            id
            name
            distributor {
              id
              name
              ... on Supplier {
                licenseNumber
                __typename
              }
              ... on Vendor {
                preferred
                __typename
              }
              ... on Manufacturer {
                country
                __typename
              }
            }
            ... on Book {
              relatedProducts {
                id
                name
                distributor {
                  id
                  name
                  ... on Supplier {
                    licenseNumber
                    __typename
                  }
                  ... on Vendor {
                    preferred
                    __typename
                  }
                  ... on Manufacturer {
                    country
                    __typename
                  }
                }
                ... on Book {
                  relatedProducts {
                    id
                    distributor {
                      id
                      name
                      ... on Manufacturer {
                        country
                        __typename
                      }
                    }
                  }
                }
              }
            }
            ... on Electronic {
              distributor {
                id
                name
                ... on Manufacturer {
                  country
                  __typename
                }
              }
            }
          }
        }
        "
        );

        let document = minify_operation(operation, &supergraph).expect("Failed to minify");

        // After minification
        insta::assert_snapshot!(
            pretty_query(document.to_string()),
            @r"
        query($id: ID!) {
          product(id: $id) {
            id
            name
            distributor {
              ...a
            }
            ... on Book {
              relatedProducts {
                id
                name
                distributor {
                  ...a
                }
                ... on Book {
                  relatedProducts {
                    id
                    distributor {
                      ...c
                    }
                  }
                }
              }
            }
            ... on Electronic {
              distributor {
                ...c
              }
            }
          }
        }

        fragment a on BusinessEntity {
          id
          name
          ... on Supplier {
            licenseNumber
            __typename
          }
          ... on Vendor {
            preferred
            __typename
          }
          ... on Manufacturer {
            ...b
          }
        }

        fragment b on Manufacturer {
          country
          __typename
        }

        fragment c on BusinessEntity {
          id
          name
          ... on Manufacturer {
            ...b
          }
        }
        "
        );
    }
}
