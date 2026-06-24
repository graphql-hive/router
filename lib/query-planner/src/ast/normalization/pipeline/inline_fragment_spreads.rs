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
                // walk the spread chain iteratively so a long acyclic chain (...F1 -> ...F2 -> ...)
                // can't blow the stack. we only recurse on a fragment body that has real content.
                // `spread` stays a borrow throughout, retargeting into `fragment_map` per link so
                // no FragmentSpread gets cloned while advancing.
                let mut spread = &spread;
                let fragment_def = loop {
                    let def = fragment_map.get(&spread.fragment_name).ok_or_else(|| {
                        NormalizationError::FragmentDefinitionNotFound {
                            fragment_name: spread.fragment_name.clone(),
                        }
                    })?;

                    // can only inline (vs wrap) when the type matches and there are no directives
                    // that would otherwise be lost.
                    let inlineable = parent_type_condition == Some(&def.type_condition)
                        && spread.directives.is_empty();

                    // pure chain link `fragment F on T { ...G }`: advance instead of recursing.
                    if inlineable {
                        if let [Selection::FragmentSpread(next)] = def.selection_set.items.as_slice()
                        {
                            spread = next;
                            continue;
                        }
                    }
                    break def;
                };

                if parent_type_condition == Some(&fragment_def.type_condition)
                    && spread.directives.is_empty()
                {
                    // same type, no directives: inline the body directly.
                    let mut inlined = fragment_def.selection_set.clone();
                    handle_selection_set(&mut inlined, fragment_map, parent_type_condition)?;
                    new_items.extend(inlined.items);
                } else {
                    // type mismatch or has directives: wrap in an inline fragment.
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
