use graphql_parser::query::{self as query_ast, Definition};

pub mod context;
pub mod error;
mod pipeline;
pub mod utils;

use crate::ast::document::NormalizedDocument;
use crate::ast::normalization::context::{NormalizationContext, RootTypes};
use crate::ast::normalization::pipeline::drop_unused_operations;
use crate::ast::normalization::pipeline::flatten_fragments;
use crate::ast::normalization::pipeline::inline_fragment_spreads;
use crate::ast::normalization::pipeline::merge_fields;
use crate::ast::normalization::pipeline::merge_inline_fragments;
use crate::ast::normalization::pipeline::normalize_fields;
use crate::ast::normalization::pipeline::{drop_duplicated_fields, drop_fragment_definitions};
use crate::state::supergraph_state::SupergraphState;
use error::NormalizationError;

/// Normalizes the operation by mutating it
pub fn normalize_operation_mut(
    supergraph: &SupergraphState,
    query: &mut query_ast::Document<'static, String>,
    operation_name: Option<&str>,
    root_types_overwrite: Option<RootTypes>,
    subgraph_name: Option<&String>,
) -> Result<(), NormalizationError> {
    let mut ctx = NormalizationContext {
        supergraph,
        operation_name,
        document: query,
        root_types: root_types_overwrite.unwrap_or_else(|| supergraph.into()),
        subgraph_name,
    };

    drop_unused_operations(&mut ctx)?;
    normalize_fields(&mut ctx)?;
    inline_fragment_spreads(&mut ctx)?;
    drop_fragment_definitions(&mut ctx)?;
    flatten_fragments(&mut ctx)?;
    merge_inline_fragments(&mut ctx)?;
    merge_fields(&mut ctx)?;
    drop_duplicated_fields(&mut ctx)?;

    Ok(())
}

pub fn normalize_operation(
    supergraph: &SupergraphState,
    query: &query_ast::Document<'static, String>,
    operation_name: Option<&str>,
) -> Result<NormalizedDocument, NormalizationError> {
    let mut query_mut = query.clone();
    normalize_operation_mut(supergraph, &mut query_mut, operation_name, None, None)?;
    let operation = query_mut.definitions.iter_mut().find_map(|def| match def {
        Definition::Operation(op) => Some(op),
        _ => None,
    });

    let op_def = operation.ok_or(NormalizationError::ExpectedTransformedOperationNotFound)?;

    Ok(create_normalized_document(
        op_def.to_owned(),
        operation_name,
    ))
}

pub fn create_normalized_document<'a, 'd>(
    operation_ast: query_ast::OperationDefinition<'d, String>,
    operation_name: Option<&'a str>,
) -> NormalizedDocument {
    NormalizedDocument {
        operation: operation_ast.into(),
        operation_name: operation_name.map(|n| n.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use graphql_parser::parse_query;

    use crate::ast::normalization::normalize_operation;
    use crate::ast::selection_item::SelectionItem;
    use crate::state::supergraph_state::SupergraphState;
    use crate::utils::parsing::parse_schema;

    fn pretty_query(query_str: String) -> String {
        format!("{}", parse_query::<&str>(&query_str).unwrap())
    }

    // TODO: remove unused variables
    // TODO: maybe we should extract variables and add additional hashmap for variables passed to the execution?

    #[test]
    fn introspection() {
        let schema = parse_schema(
            r#"
            type Query {
              words(len: Int, sep: String): String
            }
          "#,
        );

        let supergraph = SupergraphState::new(&schema);

        insta::assert_snapshot!(
            pretty_query(
                normalize_operation(
                    &supergraph,
                    &parse_query(
                        r#"
                      query {
                        a: __typename
                        words:words
                        __typename
                        __schema {
                          __typename
                        }
                      }
                "#,
                    )
                    .expect("to parse"),
                    None,
                )
                .unwrap()
                .to_string()
            ),
            @r#"
            query {
              a: __typename
              words
              __typename
              __schema {
                __typename
              }
            }
          "#
        );
    }

    #[test]
    fn normalize_fields() {
        let schema = parse_schema(
            r#"
              type Query {
                words(len: Int, sep: String): String
              }
            "#,
        );

        let supergraph = SupergraphState::new(&schema);

        insta::assert_snapshot!(
            pretty_query(
                normalize_operation(
                    &supergraph,
                    &parse_query(
                        r#"
                    query {
                      words: words(sep: ",")
                      foo: words(sep: ".", len: 10)
                    }
                  "#,
                    )
                    .expect("to parse"),
                    None,
                )
                .unwrap()
                .to_string()
            ),
            @r#"
            query {
              words(sep: ",")
              foo: words(len: 10, sep: ".")
            }
            "#
        );
    }

    #[test]
    fn drop_unused_operations() {
        let schema = parse_schema(
            r#"
              type Query {
                words: String
              }
            "#,
        );

        let supergraph = SupergraphState::new(&schema);

        insta::assert_snapshot!(
            pretty_query(
                normalize_operation(
                    &supergraph,
                    &parse_query(
                        r#"
                    query foo {
                      words
                    }

                    query bar {
                      words
                    }
                  "#,
                    )
                    .expect("to parse"),
                    Some("foo"),
                )
                .unwrap()
                .to_string()
            ),
            @r"
        query foo {
          words
        }
        "
        );
    }

    #[test]
    fn drop_duplicated_fields() {
        let schema = parse_schema(
            r#"
              type Query {
                words(len: Int, sep: String): String
              }
            "#,
        );
        let supergraph = SupergraphState::new(&schema);

        insta::assert_snapshot!(
            pretty_query(
                normalize_operation(
                    &supergraph,
                    &parse_query(
                        r#"
                    query {
                      words
                      c: words(len: 1)
                      b: words
                      a: words
                      words
                      c: words(len: 1)
                    }
                  "#,
                    )
                    .expect("to parse"),
                    None,
                )
                .unwrap()
                .to_string()
            ),
            @r#"
            query {
              words
              c: words(len: 1)
              b: words
              a: words
            }
            "#
        );
    }

    #[test]
    fn inline_fragment_spreads() {
        let schema = parse_schema(
            r#"
              interface Node
                @join__type(graph: A)
                @join__type(graph: B)
              {
                id: ID!
              }


              interface WithWarranty
                @join__type(graph: A)
                @join__type(graph: B)
              {
                warranty: Int
              }

              type Oven implements Node & WithWarranty
                @join__implements(graph: B, interface: "Node")
                @join__implements(graph: B, interface: "WithWarranty")
                @join__type(graph: A, key: "id")
                @join__type(graph: B, key: "id") {
                id: ID!
                warranty: Int
                o1: String
                o2: String
                o3: String
                toaster: Toaster
              }

              union Product
                @join__type(graph: A)
                @join__unionMember(graph: A, member: "Oven")
                @join__unionMember(graph: A, member: "Toaster")
               = Oven | Toaster

               type Query
                 @join__type(graph: A)
                 @join__type(graph: B)
               {
                 products: [Product] @join__field(graph: A)
                 node(id: ID!): Node @join__field(graph: A)
                 nodes: [Node] @join__field(graph: A)
                 toasters: [Toaster] @join__field(graph: A)
               }

               type Toaster implements Node & WithWarranty
                 @join__implements(graph: A, interface: "Node")
                 @join__implements(graph: A, interface: "WithWarranty")
                 @join__type(graph: A, key: "id")
               {
                id: ID!
                warranty: Int
                t1: String
                t2: String
                t3: String
                oven: Oven
              }
            "#,
        );
        let supergraph = SupergraphState::new(&schema);

        insta::assert_snapshot!(
            pretty_query(
                normalize_operation(
                    &supergraph,
                    &parse_query(
                        r#"
                        query {
                          products {
                            ...ToasterFragment
                            ... on Toaster {
                              t1
                              t2
                              oven {
                                warranty
                                o1
                                o2
                              }
                            }
                          }
                        }
                        fragment ToasterFragment on Toaster {
                          t1
                          oven {
                            o1
                            ... on Oven {
                              ... on Oven {
                                o3
                              }
                            }
                            ...OvenFragment
                            ...OvenFragment

                          }
                        }
                        fragment OvenFragment on Oven {
                          toaster {
                            warranty
                            t3
                          }
                          o3
                        }
                  "#,
                    )
                    .expect("to parse"),
                    None,
                )
                .unwrap()
                .to_string()
            ),
            @r"
        query {
          products {
            ... on Toaster {
              t1
              oven {
                o1
                o3
                toaster {
                  warranty
                  t3
                }
                warranty
                o2
              }
              t2
            }
          }
        }
        ",
        );

        insta::assert_snapshot!(
            pretty_query(
                normalize_operation(
                    &supergraph,
                    &parse_query(
                        r#"
                        query {
                          products {
                            ... on Node {
                              id
                            }
                            ... on WithWarranty {
                              warranty
                            }
                          }
                        }
                  "#,
                    )
                    .expect("to parse"),
                    None,
                )
                .unwrap()
                .to_string()
            ),
            @r"
        query {
          products {
            ... on Oven {
              id
              warranty
            }
            ... on Toaster {
              id
              warranty
            }
          }
        }
        ",
        );

        insta::assert_snapshot!(
            pretty_query(
                normalize_operation(
                    &supergraph,
                    &parse_query(
                        r#"
                        query {
                          toasters {
                            ...ToasterFragment
                            ...NodeFragment
                          }
                        }

                        fragment ToasterFragment on Toaster {
                          id
                        }

                        fragment NodeFragment on Node {
                          id
                          __typename
                        }
                  "#,
                    )
                    .expect("to parse"),
                    None,
                )
                .unwrap()
                .to_string()
            ),
            @r"
            query {
              toasters {
                id
                __typename
              }
            }
        ",
        );
    }

    // Makes sure that fields arguments are normalized and sorted correctly, because hashing relies on the order of arguments.
    #[test]
    fn normalize_fields_args() {
        let schema = parse_schema(
            r#"
              type Query {
                words(len: Int, sep: String): String
              }
            "#,
        );

        let supergraph = SupergraphState::new(&schema);
        let r = normalize_operation(
            &supergraph,
            &parse_query(
                r#"
            query {
              one: words(sep: ".", len: 10)
              two: words(len: 10, sep: ".")
            }
          "#,
            )
            .expect("to parse"),
            None,
        )
        .unwrap();

        match (
            &r.operation.selection_set.items[0],
            &r.operation.selection_set.items[1],
        ) {
            (SelectionItem::Field(f1), SelectionItem::Field(f2)) => {
                assert_eq!(f1.arguments_hash(), f2.arguments_hash());
            }
            _ => panic!("Unexpected selection items"),
        }

        insta::assert_snapshot!(
            pretty_query(r.to_string()),
            @r###"
        query {
          one: words(len: 10, sep: ".")
          two: words(len: 10, sep: ".")
        }
        "###
        );
    }

    #[test]
    fn mutation() {
        let schema = parse_schema(
            r#"
                  type Mutation {
                    createProduct(name: String!): Product
                  }

                  type Query {
                    products: [Product]
                  }

                  type Product {
                    id: ID!
                    name: String
                  }
                "#,
        );
        let supergraph = SupergraphState::new(&schema);

        insta::assert_snapshot!(
            pretty_query(
                normalize_operation(
                    &supergraph,
                    &parse_query(
                        r#"
                              mutation NewProduct($name: String!) {
                                createProduct(name: $name) {
                                  id
                                  name
                                }
                              }
                            "#,
                    )
                    .expect("to parse"),
                    None,
                )
                .unwrap()
                .to_string()
            ),
            @r"
            mutation NewProduct($name: String!) {
              createProduct(name: $name) {
                id
                name
              }
            }
            ",
        );
    }

    #[test]
    fn type_expansion() {
        let schema_str = std::fs::read_to_string(
            "./fixture/tests/corrupted-supergraph-node-id.supergraph.graphql",
        )
        .expect("Unable to read supergraph");
        let schema = parse_schema(&schema_str);
        let supergraph = SupergraphState::new(&schema);

        insta::assert_snapshot!(
            pretty_query(
                normalize_operation(
                    &supergraph,
                    &parse_query(
                        r#"
                          query nodeid($id: ID!) {
                            node(id: $id) {
                              id
                            }
                          }
                        "#,
                    )
                    .expect("to parse"),
                    None,
                )
                .unwrap()
                .to_string()
            ),
            @r"
        query nodeid($id: ID!) {
          node(id: $id) {
            ... on Account {
              id
            }
            ... on Chat {
              id
            }
          }
        }
        ",
        );

        insta::assert_snapshot!(
            pretty_query(
                normalize_operation(
                    &supergraph,
                    &parse_query(
                        r#"
                          query nodeid($id: ID!) {
                            node(id: $id) {
                              ... on Chat {
                                id
                              }
                            }
                          }
                        "#,
                    )
                    .expect("to parse"),
                    None,
                )
                .unwrap()
                .to_string()
            ),
            @r"
            query nodeid($id: ID!) {
              node(id: $id) {
                ... on Chat {
                  id
                }
              }
            }
            ",
        );
    }

    #[test]
    fn type_expansion_2() {
        let schema_str =
            std::fs::read_to_string("./fixture/tests/requires-with-fragments.supergraph.graphql")
                .expect("Unable to read supergraph");
        let schema = parse_schema(&schema_str);
        let supergraph = SupergraphState::new(&schema);

        insta::assert_snapshot!(
            pretty_query(
                normalize_operation(
                    &supergraph,
                    &parse_query(
                        r#"
                          query {
                            userFromA {
                              profile {
                                displayName
                                ... on Account {
                                  accountType
                                }
                              }
                            }
                          }
                        "#,
                    )
                    .expect("to parse"),
                    None,
                )
                .unwrap()
                .to_string()
            ),
            @r"
        query {
          userFromA {
            profile {
              displayName
              ... on AdminAccount {
                accountType
              }
              ... on GuestAccount {
                accountType
              }
            }
          }
        }
        ",
        );

        insta::assert_snapshot!(
            pretty_query(
                normalize_operation(
                    &supergraph,
                    &parse_query(
                        r#"
                          query {
                            userFromA {
                              profile {
                                displayName
                                ... on Account {
                                  accountType
                                  ... on AdminAccount {
                                    adminLevel
                                  }
                                  ... on GuestAccount {
                                    guestToken
                                  }
                                }
                              }
                            }
                          }
                        "#,
                    )
                    .expect("to parse"),
                    None,
                )
                .unwrap()
                .to_string()
            ),
            @r"
        query {
          userFromA {
            profile {
              displayName
              ... on AdminAccount {
                accountType
                adminLevel
              }
              ... on GuestAccount {
                accountType
                guestToken
              }
            }
          }
        }
        ",
        );
    }
}
