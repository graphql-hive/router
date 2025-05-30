use std::collections::HashMap;

use graphql_parser::query as parser;

use crate::{
    ast::{
        document::NormalizedDocument,
        operation::{OperationDefinition, VariableDefinition},
        selection_item::SelectionItem,
        selection_set::{FieldSelection, InlineFragmentSelection, SelectionSet},
    },
    state::supergraph_state::OperationKind,
};

type KnownFragments<'a> = HashMap<&'a String, &'a parser::FragmentDefinition<'static, String>>;

pub fn prepare_document(
    original_document: parser::Document<'static, String>,
    operation_name: Option<&str>,
) -> NormalizedDocument {
    let known_fragments = original_document
        .definitions
        .iter()
        .filter_map(|definition| {
            if let parser::Definition::Fragment(fragment) = definition {
                Some((&fragment.name, fragment))
            } else {
                None
            }
        })
        .collect::<KnownFragments>();

    let mut operations: Vec<OperationDefinition> = vec![];

    for definition in &original_document.definitions {
        if let parser::Definition::Operation(operation) = definition {
            let operation_definition = match &operation {
                parser::OperationDefinition::Query(query) => OperationDefinition {
                    name: query.name.clone(),
                    operation_kind: Some(OperationKind::Query),
                    variable_definitions: transform_variables(&query.variable_definitions),
                    selection_set: transform_selection_set(&query.selection_set, &known_fragments),
                },
                parser::OperationDefinition::SelectionSet(ss) => OperationDefinition {
                    name: None,
                    operation_kind: Some(OperationKind::Query),
                    variable_definitions: None,
                    selection_set: transform_selection_set(ss, &known_fragments),
                },
                parser::OperationDefinition::Mutation(mutation) => OperationDefinition {
                    name: mutation.name.clone(),
                    operation_kind: Some(OperationKind::Mutation),
                    variable_definitions: transform_variables(&mutation.variable_definitions),
                    selection_set: transform_selection_set(
                        &mutation.selection_set,
                        &known_fragments,
                    ),
                },
                parser::OperationDefinition::Subscription(subscription) => OperationDefinition {
                    name: subscription.name.clone(),
                    operation_kind: Some(OperationKind::Subscription),
                    variable_definitions: transform_variables(&subscription.variable_definitions),
                    selection_set: transform_selection_set(
                        &subscription.selection_set,
                        &known_fragments,
                    ),
                },
            };

            operations.push(operation_definition);
        }
    }

    NormalizedDocument {
        operations,
        operation_name: operation_name.map(|name| name.to_string()),
        original_document,
    }
}

fn transform_selection_set(
    selection_set: &parser::SelectionSet<'static, String>,
    known_fragments: &KnownFragments,
) -> SelectionSet {
    let mut transformed_selection_set = SelectionSet::default();

    for selection in &selection_set.items {
        match selection {
            parser::Selection::Field(field) => {
                transformed_selection_set
                    .items
                    .push(SelectionItem::Field(FieldSelection {
                        name: field.name.clone(),
                        alias: field.alias.clone(),
                        arguments: if field.arguments.is_empty() {
                            None
                        } else {
                            Some((&field.arguments).into())
                        },
                        selections: transform_selection_set(&field.selection_set, known_fragments),
                    }));
            }
            parser::Selection::InlineFragment(inline_fragment) => {
                transformed_selection_set
                    .items
                    .push(SelectionItem::InlineFragment(InlineFragmentSelection {
                        type_condition: inline_fragment
                            .type_condition
                            .as_ref()
                            .map(extract_type_condition)
                            .unwrap(),
                        selections: transform_selection_set(
                            &inline_fragment.selection_set,
                            known_fragments,
                        ),
                    }));
            }
            parser::Selection::FragmentSpread(fragment_spread) => {
                match known_fragments.get(&fragment_spread.fragment_name) {
                    Some(fragment_definition) => {
                        let type_condition =
                            extract_type_condition(&fragment_definition.type_condition);
                        transformed_selection_set
                            .items
                            .push(SelectionItem::InlineFragment(InlineFragmentSelection {
                                type_condition,
                                selections: transform_selection_set(
                                    &fragment_definition.selection_set,
                                    known_fragments,
                                ),
                            }));
                    }
                    None => {
                        unimplemented!("fragment is not defined")
                    }
                }
            }
        }
    }

    transformed_selection_set
}

fn extract_type_condition(type_condition: &parser::TypeCondition<'static, String>) -> String {
    match type_condition {
        parser::TypeCondition::On(v) => v.to_string(),
    }
}

fn transform_variables(
    parser_variables: &Vec<parser::VariableDefinition<'static, String>>,
) -> Option<Vec<VariableDefinition>> {
    match parser_variables.len() {
        0 => None,
        _ => Some(parser_variables.iter().map(|pv| pv.into()).collect()),
    }
}

#[cfg(test)]
mod tests {
    use graphql_parser::parse_query;

    use crate::utils::operation_utils::prepare_document;

    #[test]
    fn flatten_fragment_spreads() {
        let parsed = parse_query(
            r#"
      query {
        test {
          ...TestFragment
          ... on TestType {
            field10
            field21
          }
          otherField
          nested {
            ...OtherFragment
          }
          nested2 {
            ...OtherFragment
            sibling
          }
        }
      }

      fragment TestFragment on TestType {
        field1
        field2
      }

      fragment OtherFragment on SomeType {
        field3
        nested {
          other
        }
        field4
      }
    "#,
        )
        .expect("to parse");

        let transformed = prepare_document(parsed, None);
        insta::assert_snapshot!(transformed.to_string(), @"query{test{...on TestType{field1 field2} ...on TestType{field10 field21} otherField nested{...on SomeType{field3 nested{other} field4}} nested2{...on SomeType{field3 nested{other} field4} sibling}}}");
    }
}
