use std::collections::HashMap;

use graphql_tools::parser::query::{
    Definition, FragmentDefinition, InlineFragment, Mutation, OperationDefinition, Query,
    Selection, SelectionSet, Subscription, TypeCondition,
};

use crate::ast::normalization::{context::NormalizationContext, error::NormalizationError};

#[inline]
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
                    handle_selection_set(selection_set, &fragment_map, None)?;
                }
                OperationDefinition::Query(Query { selection_set, .. }) => {
                    handle_selection_set(selection_set, &fragment_map, None)?;
                }
                OperationDefinition::Mutation(Mutation { selection_set, .. }) => {
                    handle_selection_set(selection_set, &fragment_map, None)?;
                }
                OperationDefinition::Subscription(Subscription { selection_set, .. }) => {
                    handle_selection_set(selection_set, &fragment_map, None)?;
                }
            },
            Definition::Fragment(frag_def) => {
                handle_selection_set(
                    &mut frag_def.selection_set,
                    &fragment_map,
                    Some(&frag_def.type_condition),
                )?;
            }
        }
    }

    Ok(())
}

#[inline]
fn handle_selection_set<'a>(
    selection_set: &mut SelectionSet<'a, String>,
    fragment_map: &HashMap<String, FragmentDefinition<'a, String>>,
    parent_type_condition: Option<&TypeCondition<'a, String>>,
) -> Result<(), NormalizationError> {
    let old_items = std::mem::take(&mut selection_set.items);
    let mut new_items = Vec::with_capacity(old_items.len());

    for selection in old_items {
        match selection {
            Selection::Field(mut field) => {
                handle_selection_set(
                    &mut field.selection_set,
                    fragment_map,
                    parent_type_condition,
                )?;
                new_items.push(Selection::Field(field));
            }
            Selection::FragmentSpread(spread) => {
                let fragment_def = fragment_map.get(&spread.fragment_name).ok_or_else(|| {
                    NormalizationError::FragmentDefinitionNotFound {
                        fragment_name: spread.fragment_name.clone(),
                    }
                })?;

                if parent_type_condition == Some(&fragment_def.type_condition) {
                    // If the fragment's type condition matches the top type condition,
                    // we can inline its selections directly.
                    let mut selection_set = fragment_def.selection_set.clone();
                    handle_selection_set(&mut selection_set, fragment_map, parent_type_condition)?;
                    new_items.extend(selection_set.items);
                } else {
                    let mut inline_fragment = InlineFragment {
                        position: spread.position,
                        type_condition: Some(fragment_def.type_condition.clone()),
                        directives: spread.directives.clone(),
                        selection_set: fragment_def.selection_set.clone(),
                    };

                    handle_selection_set(
                        &mut inline_fragment.selection_set,
                        fragment_map,
                        inline_fragment.type_condition.as_ref(),
                    )?;

                    new_items.push(Selection::InlineFragment(inline_fragment));
                }
            }
            Selection::InlineFragment(mut inline_fragment) => {
                handle_selection_set(
                    &mut inline_fragment.selection_set,
                    fragment_map,
                    inline_fragment.type_condition.as_ref(),
                )?;
                new_items.push(Selection::InlineFragment(inline_fragment));
            }
        }
    }
    selection_set.items = new_items;

    Ok(())
}
