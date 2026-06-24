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
                    // Crossing a field boundary resets the type condition context
                    None,
                )?;
                new_items.push(Selection::Field(field));
            }
            Selection::FragmentSpread(spread) => {
                // iteratively resolve the fragment spread chain to avoid stack overflow on long
                // acyclic chains. each iteration peels one same-type same-spread level off the chain
                // until we hit a field boundary, a type mismatch, a directive, or a leaf.
                let mut current_spread_name = spread.fragment_name.clone();
                let mut current_directives = spread.directives.clone();
                let mut current_position = spread.position;
                let current_parent_tc = parent_type_condition.cloned();

                loop {
                    let fragment_def = fragment_map.get(&current_spread_name).ok_or_else(|| {
                        NormalizationError::FragmentDefinitionNotFound {
                            fragment_name: current_spread_name.clone(),
                        }
                    })?;

                    if current_parent_tc.as_ref() == Some(&fragment_def.type_condition)
                        && current_directives.is_empty()
                    {
                        // same type, no directives: the original code would inline and recurse.
                        // check if the entire body is a single fragment spread with no fields,
                        // in which case we can just advance the chain iteratively.
                        let items = &fragment_def.selection_set.items;
                        if items.len() == 1 {
                            if let Selection::FragmentSpread(next_spread) = &items[0] {
                                // pure chain link: advance without recursing
                                current_spread_name = next_spread.fragment_name.clone();
                                current_directives = next_spread.directives.clone();
                                current_position = next_spread.position;
                                // parent type condition stays the same
                                continue;
                            }
                        }
                        // body has real content: inline it and recurse normally (bounded by
                        // field depth, which is bounded by max_depth validation)
                        let mut inlined = fragment_def.selection_set.clone();
                        handle_selection_set(
                            &mut inlined,
                            fragment_map,
                            current_parent_tc.as_ref(),
                        )?;
                        new_items.extend(inlined.items);
                    } else {
                        // type mismatch or has directives: wrap in an inline fragment
                        let mut inline_fragment = InlineFragment {
                            position: current_position,
                            type_condition: Some(fragment_def.type_condition.clone()),
                            directives: current_directives.clone(),
                            selection_set: fragment_def.selection_set.clone(),
                        };
                        handle_selection_set(
                            &mut inline_fragment.selection_set,
                            fragment_map,
                            inline_fragment.type_condition.as_ref(),
                        )?;
                        new_items.push(Selection::InlineFragment(inline_fragment));
                    }
                    break;
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
