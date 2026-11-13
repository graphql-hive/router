use crate::query_planner::ast::flat::plan_document::PlanDocument;
use crate::query_planner::ast::minification::error::MinificationError;
use crate::query_planner::ast::minification::stats::Stats;
use crate::query_planner::ast::minification::transform::transform_operation;
use crate::query_planner::state::supergraph_state::{OperationKind, SupergraphState};

pub mod error;
mod selection_id;
mod stats;
mod transform;

/// Moves repeated selection sets of `document`'s operation into fragments. `document` comes
/// without fragments, its lists each its own.
pub fn minify(
    document: &mut PlanDocument,
    supergraph: &SupergraphState,
) -> Result<(), MinificationError> {
    let root_type_name = get_root_type_name(document.kind.as_ref(), supergraph)?;
    let stats = Stats::from_operation(
        &document.arena,
        document.selections,
        supergraph,
        root_type_name,
    )?;
    transform_operation(supergraph, stats, root_type_name, document)
}

fn get_root_type_name<'a>(
    kind: Option<&OperationKind>,
    supergraph: &'a SupergraphState,
) -> Result<&'a str, MinificationError> {
    Ok(match kind {
        None => &supergraph.query_type,
        Some(OperationKind::Query) => &supergraph.query_type,
        Some(OperationKind::Mutation) => supergraph
            .mutation_type
            .as_ref()
            .ok_or_else(|| MinificationError::TypeForOperationNotFound("mutation".to_string()))?,
        Some(OperationKind::Subscription) => {
            supergraph.subscription_type.as_ref().ok_or_else(|| {
                MinificationError::TypeForOperationNotFound("subscription".to_string())
            })?
        }
    })
}

#[cfg(test)]
mod tests {
    use graphql_tools::parser::parse_query;
    use graphql_tools::parser::query::Document;

    use crate::query_planner::ast::flat::plan_document::PlanDocument;
    use crate::query_planner::ast::minification::minify;
    use crate::query_planner::ast::normalization::normalize_operation;
    use crate::query_planner::ast::operation::OperationDefinition;
    use crate::query_planner::state::supergraph_state::SupergraphState;
    use crate::query_planner::utils::parsing::parse_schema;

    fn minify_operation(
        operation: OperationDefinition,
        supergraph: &SupergraphState,
    ) -> PlanDocument {
        let mut document =
            PlanDocument::from_tree(&crate::query_planner::ast::document::Document {
                operation,
                fragments: vec![],
            });
        minify(&mut document, supergraph).expect("Failed to minify");
        document
    }

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
        let operation: OperationDefinition = normalized.operation_tree();

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

        let document = minify_operation(operation, &supergraph);

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

    #[test]
    fn selection_sets_in_another_order_get_their_own_fragment() {
        let schema = parse_schema(
            r#"
            type Query {
              a: Text
              b: Text
              c: Text
            }

            type Text {
              locale: String
              value: String
            }
        "#,
        );
        let supergraph = SupergraphState::new(&schema);
        let document: Document<'static, String> = parse_query(
            r#"
        {
          a { locale value }
          b { value locale }
          c { value locale }
          d: a { locale value }
        }
        "#,
        )
        .expect("Failed to parse query");
        let operation = normalize_operation(&supergraph, &document, None)
            .expect("Failed to normalized")
            .operation_tree();
        let document = minify_operation(operation, &supergraph);

        // a subgraph answers in the order the fragment has: sharing one fragment between
        // `{ locale value }` and `{ value locale }` would give the router fields to reorder
        insta::assert_snapshot!(pretty_query(document.to_string()), @r"
        {
          a {
            ...a
          }
          b {
            ...b
          }
          c {
            ...b
          }
          d: a {
            ...a
          }
        }

        fragment a on Text {
          locale
          value
        }

        fragment b on Text {
          value
          locale
        }
        ");
    }
}
