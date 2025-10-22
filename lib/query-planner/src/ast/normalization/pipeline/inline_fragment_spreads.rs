use std::collections::HashMap;

use graphql_parser::query::{
    Definition, FragmentDefinition, InlineFragment, Mutation, OperationDefinition, Query,
    Selection, SelectionSet, Subscription, TypeCondition
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
                handle_selection_set(&mut frag_def.selection_set, &fragment_map, None)?;
            }
        }
    }

    Ok(())
}

#[inline]
fn handle_selection_set<'a>(
    selection_set: &mut SelectionSet<'a, String>,
    fragment_map: &HashMap<String, FragmentDefinition<'a, String>>,
    top_type_condition: Option<String>,
) -> Result<(), NormalizationError> {
    let old_items = std::mem::take(&mut selection_set.items);
    let mut new_items = Vec::with_capacity(old_items.len());

    for selection in old_items {
        match selection {
            Selection::Field(mut field) => {
                handle_selection_set(&mut field.selection_set, fragment_map, None)?;
                new_items.push(Selection::Field(field));
            }
            Selection::FragmentSpread(spread) => {
                let fragment_def = fragment_map.get(&spread.fragment_name).ok_or_else(|| {
                    NormalizationError::FragmentDefinitionNotFound {
                        fragment_name: spread.fragment_name.clone(),
                    }
                })?;
                let type_condition = match &fragment_def.type_condition {
                    TypeCondition::On(name) => name.to_string(),
                };

                let mut new_selection_set = fragment_def.selection_set.clone();
                if let Some(ref top_type_condition) = top_type_condition {
                    if top_type_condition == &type_condition {
                        // If the fragment's type condition matches the top type condition,
                        // we can inline its selections directly.
                        handle_selection_set(&mut new_selection_set, fragment_map, Some(top_type_condition.clone()))?;
                        new_items.extend(new_selection_set.items);
                        continue;
                    }
                }
                let mut inline_fragment = InlineFragment {
                    position: spread.position,
                    type_condition: Some(fragment_def.type_condition.clone()),
                    directives: spread.directives.clone(),
                    selection_set: new_selection_set,
                };

                handle_selection_set(&mut inline_fragment.selection_set, fragment_map, Some(type_condition))?;

                new_items.push(Selection::InlineFragment(inline_fragment));
            }
            Selection::InlineFragment(mut inline_fragment) => {
                let type_condition = if let Some(TypeCondition::On(name)) = &inline_fragment.type_condition {
                    Some(name.to_string())
                } else {
                    None
                };
                handle_selection_set(&mut inline_fragment.selection_set, fragment_map, type_condition)?;
                new_items.push(Selection::InlineFragment(inline_fragment));
            }
        }
    }
    selection_set.items = new_items;

    Ok(())
}
