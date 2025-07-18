use graphql_parser::query::{
    Definition, FragmentDefinition, InlineFragment, Mutation, OperationDefinition, Query,
    Selection, SelectionSet, Subscription,
};

use crate::ast::normalization::{context::NormalizationContext, error::NormalizationError};
use hashbrown::HashMap;

pub fn inline_fragment_spreads(ctx: &mut NormalizationContext) -> Result<(), NormalizationError> {
    let mut fragment_map: HashMap<String, FragmentDefinition<'static, String>> = HashMap::new();
    for definition in &ctx.document.definitions {
        if let Definition::Fragment(frag_def) = definition {
            fragment_map.insert(frag_def.name.clone(), frag_def.clone());
        }
    }

    for definition in &mut ctx.document.definitions {
        match definition {
            Definition::Operation(op_def) => match op_def {
                OperationDefinition::SelectionSet(selection_set) => {
                    handle_selection_set(selection_set, &fragment_map)?;
                }
                OperationDefinition::Query(Query { selection_set, .. }) => {
                    handle_selection_set(selection_set, &fragment_map)?;
                }
                OperationDefinition::Mutation(Mutation { selection_set, .. }) => {
                    handle_selection_set(selection_set, &fragment_map)?;
                }
                OperationDefinition::Subscription(Subscription { selection_set, .. }) => {
                    handle_selection_set(selection_set, &fragment_map)?;
                }
            },
            Definition::Fragment(frag_def) => {
                handle_selection_set(&mut frag_def.selection_set, &fragment_map)?;
            }
        }
    }

    Ok(())
}

fn handle_selection_set<'a>(
    selection_set: &mut SelectionSet<'a, String>,
    fragment_map: &HashMap<String, FragmentDefinition<'a, String>>,
) -> Result<(), NormalizationError> {
    let old_items = std::mem::take(&mut selection_set.items);
    let mut new_items = Vec::with_capacity(old_items.len());

    for selection in old_items {
        match selection {
            Selection::Field(mut field) => {
                handle_selection_set(&mut field.selection_set, fragment_map)?;
                new_items.push(Selection::Field(field));
            }
            Selection::FragmentSpread(spread) => {
                let fragment_def = fragment_map.get(&spread.fragment_name).ok_or_else(|| {
                    NormalizationError::FragmentDefinitionNotFound {
                        fragment_name: spread.fragment_name.clone(),
                    }
                })?;

                let mut inline_fragment = InlineFragment {
                    position: spread.position,
                    type_condition: Some(fragment_def.type_condition.clone()),
                    directives: spread.directives.clone(),
                    selection_set: fragment_def.selection_set.clone(),
                };

                handle_selection_set(&mut inline_fragment.selection_set, fragment_map)?;

                new_items.push(Selection::InlineFragment(inline_fragment));
            }
            Selection::InlineFragment(mut inline_fragment) => {
                handle_selection_set(&mut inline_fragment.selection_set, fragment_map)?;
                new_items.push(Selection::InlineFragment(inline_fragment));
            }
        }
    }
    selection_set.items = new_items;

    Ok(())
}
