use std::mem;

use graphql_parser::query::Value;
use graphql_parser::query::{self as query_ast, Definition};

mod context;
mod error;
mod pipeline;
mod utils;

use crate::ast::arguments::ArgumentsMap;
use crate::ast::normalization::context::NormalizationContext;
use crate::ast::normalization::pipeline::drop_duplicated_fields;
use crate::ast::normalization::pipeline::drop_unused_operations;
use crate::ast::normalization::pipeline::flatten_fragments;
use crate::ast::normalization::pipeline::inline_fragment_spreads;
use crate::ast::normalization::pipeline::merge_fields;
use crate::ast::normalization::pipeline::merge_inline_fragments;
use crate::ast::normalization::pipeline::normalize_fields;
use crate::ast::normalization::utils::extract_type_condition;
use crate::ast::operation::{OperationDefinition, VariableDefinition};
use crate::ast::selection_item::SelectionItem;
use crate::ast::selection_set::{FieldSelection, InlineFragmentSelection, SelectionSet};
use crate::state::supergraph_state::OperationKind;
use crate::{ast::document::NormalizedDocument, consumer_schema::ConsumerSchema};
use error::NormalizationError;

pub fn normalize_operation(
    schema: &ConsumerSchema,
    query: &query_ast::Document<'static, String>,
    operation_name: Option<&str>,
) -> Result<NormalizedDocument, NormalizationError> {
    let mut document = query.clone();
    let mut ctx = NormalizationContext {
        operation_name,
        document: &mut document,
        schema: &schema.document,
    };

    drop_unused_operations(&mut ctx)?;
    normalize_fields(&mut ctx)?;
    inline_fragment_spreads(&mut ctx)?;
    flatten_fragments(&mut ctx)?;
    merge_inline_fragments(&mut ctx)?;
    merge_fields(&mut ctx)?;
    drop_duplicated_fields(&mut ctx)?;

    // drops fragment definitions
    let operation = ctx
        .document
        .definitions
        .iter_mut()
        .find_map(|def| match def {
            Definition::Operation(op) => Some(op),
            _ => None,
        });

    let op_def = operation.ok_or(NormalizationError::ExpectedTransformedOperationNotFound)?;

    Ok(create_normalized_document(op_def, operation_name))
}

pub fn create_normalized_document<'a, 'd>(
    operation_ast: &'a mut query_ast::OperationDefinition<'d, String>,
    operation_name: Option<&'a str>,
) -> NormalizedDocument {
    NormalizedDocument {
        operation: match operation_ast {
            query_ast::OperationDefinition::Query(query) => OperationDefinition {
                name: mem::take(&mut query.name),
                operation_kind: Some(OperationKind::Query),
                variable_definitions: transform_variables(&mut query.variable_definitions),
                selection_set: transform_selection_set(&mut query.selection_set),
            },
            query_ast::OperationDefinition::SelectionSet(s) => OperationDefinition {
                name: None,
                operation_kind: Some(OperationKind::Query),
                variable_definitions: None,
                selection_set: transform_selection_set(s),
            },
            query_ast::OperationDefinition::Mutation(mutation) => OperationDefinition {
                name: mem::take(&mut mutation.name),
                operation_kind: Some(OperationKind::Mutation),
                variable_definitions: transform_variables(&mut mutation.variable_definitions),
                selection_set: transform_selection_set(&mut mutation.selection_set),
            },
            query_ast::OperationDefinition::Subscription(subscription) => OperationDefinition {
                name: mem::take(&mut subscription.name),
                operation_kind: Some(OperationKind::Subscription),
                variable_definitions: transform_variables(&mut subscription.variable_definitions),
                selection_set: transform_selection_set(&mut subscription.selection_set),
            },
        },
        operation_name: operation_name.map(|n| n.to_string()),
    }
}

fn transform_selection_set<'a, 'd>(
    selection_set: &'a mut query_ast::SelectionSet<'d, String>,
) -> SelectionSet {
    let mut transformed_selection_set = SelectionSet {
        items: Vec::with_capacity(selection_set.items.len()),
    };

    for selection in &mut selection_set.items {
        match selection {
            query_ast::Selection::Field(field) => {
                let mut skip_if: Option<String> = None;
                let mut include_if: Option<String> = None;
                for directive in &field.directives {
                    match directive.name.as_str() {
                        "skip" => {
                            let if_arg = directive
                                .arguments
                                .iter()
                                .find(|(name, _value)| name == "if")
                                .map(|(_name, value)| value);
                            match if_arg {
                                Some(query_ast::Value::Boolean(true)) => {
                                    continue;
                                }
                                Some(query_ast::Value::Variable(var_name)) => {
                                    skip_if = Some(var_name.to_string());
                                }
                                _ => {}
                            }
                        }
                        "include" => {
                            let if_arg = directive
                                .arguments
                                .iter()
                                .find(|(name, _value)| name == "if")
                                .map(|(_name, value)| value);
                            match if_arg {
                                Some(query_ast::Value::Boolean(false)) => {
                                    continue;
                                }
                                Some(query_ast::Value::Variable(var_name)) => {
                                    include_if = Some(var_name.to_string());
                                }
                                _ => {}
                            }
                        }
                        _ => {}
                    }
                }
                transformed_selection_set
                    .items
                    .push(SelectionItem::Field(FieldSelection {
                        name: mem::take(&mut field.name),
                        alias: mem::take(&mut field.alias),
                        arguments: transform_arguments(&mut field.arguments),
                        selections: transform_selection_set(&mut field.selection_set),
                        skip_if,
                        include_if,
                    }));
            }
            query_ast::Selection::InlineFragment(inline_fragment) => {
                transformed_selection_set
                    .items
                    .push(SelectionItem::InlineFragment(InlineFragmentSelection {
                        type_condition: inline_fragment
                            .type_condition
                            .as_ref()
                            .map(extract_type_condition)
                            .expect(
                                "Inline fragment in normalized query must have a type condition",
                            ),
                        selections: transform_selection_set(&mut inline_fragment.selection_set),
                    }));
            }
            query_ast::Selection::FragmentSpread(_) => {
                panic!("Normalized query document should not have fragment spreads")
            }
        }
    }

    transformed_selection_set
}

fn transform_variables<'a, 'd>(
    parser_variables: &'a Vec<query_ast::VariableDefinition<'d, String>>,
) -> Option<Vec<VariableDefinition>> {
    match parser_variables.len() {
        0 => None,
        _ => {
            let mut variables: Vec<VariableDefinition> = Vec::with_capacity(parser_variables.len());
            for variable in parser_variables {
                variables.push(variable.into())
            }

            Some(variables)
        }
    }
}

fn transform_arguments<'a, 'd>(
    arguments: &'a mut Vec<(String, Value<'d, String>)>,
) -> Option<ArgumentsMap> {
    if arguments.is_empty() {
        None
    } else {
        Some((arguments).into())
    }
}

#[cfg(test)]
mod tests {
    use graphql_parser::parse_query;
    use graphql_parser::parse_schema;

    use crate::ast::normalization::normalize_operation;
    use crate::consumer_schema::ConsumerSchema;

    fn pretty_query(query_str: String) -> String {
        format!("{}", parse_query::<&str>(&query_str).unwrap())
    }

    // TODO: remove unused variables
    // TODO: maybe we should extract variables and add additional hashmap for variables passed to the execution?

    #[test]
    fn normalize_fields() {
        let schema = parse_schema(
            r#"
              type Query {
                words(len: Int, sep: String): String
              }
            "#,
        )
        .expect("to parse");

        let consumer_schema = ConsumerSchema::new_from_supergraph(&schema);

        insta::assert_snapshot!(
            pretty_query(
                normalize_operation(
                    &consumer_schema,
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
        )
        .expect("to parse");

        let consumer_schema = ConsumerSchema::new_from_supergraph(&schema);

        insta::assert_snapshot!(
            pretty_query(
                normalize_operation(
                    &consumer_schema,
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
        )
        .expect("to parse");

        let consumer_schema = ConsumerSchema::new_from_supergraph(&schema);

        insta::assert_snapshot!(
            pretty_query(
                normalize_operation(
                    &consumer_schema,
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
              interface Node {
                id: ID!
              }

              interface WithWarranty {
                warranty: Int
              }

              type Oven implements Node & WithWarranty {
                id: ID!
                warranty: Int
                o1: String
                o2: String
                o3: String
                toaster: Toaster
              }

              union Product = Oven | Toaster

              type Query {
                products: [Product]
                node(id: ID!): Node
                nodes: [Node]
                toasters: [Toaster]
              }

              type Toaster implements Node & WithWarranty {
                id: ID!
                warranty: Int
                t1: String
                t2: String
                t3: String
                oven: Oven
              }
            "#,
        )
        .expect("to parse");

        let consumer_schema = ConsumerSchema::new_from_supergraph(&schema);

        insta::assert_snapshot!(
            pretty_query(
                normalize_operation(
                    &consumer_schema,
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
                    &consumer_schema,
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
    }
}
