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
        // fresh per top-level definition: tracks fragment names currently being expanded on
        // the active inline/wrap path, so any cyclic expansion - regardless of shape - is
        // caught before it recurses forever.
        let mut active_fragments = Vec::new();
        match definition {
            Definition::Operation(op_def) => match op_def {
                OperationDefinition::SelectionSet(selection_set) => {
                    handle_selection_set(
                        selection_set,
                        &fragment_map,
                        None,
                        &mut active_fragments,
                    )?;
                }
                OperationDefinition::Query(Query { selection_set, .. }) => {
                    handle_selection_set(
                        selection_set,
                        &fragment_map,
                        None,
                        &mut active_fragments,
                    )?;
                }
                OperationDefinition::Mutation(Mutation { selection_set, .. }) => {
                    handle_selection_set(
                        selection_set,
                        &fragment_map,
                        None,
                        &mut active_fragments,
                    )?;
                }
                OperationDefinition::Subscription(Subscription { selection_set, .. }) => {
                    handle_selection_set(
                        selection_set,
                        &fragment_map,
                        None,
                        &mut active_fragments,
                    )?;
                }
            },
            Definition::Fragment(frag_def) => {
                handle_selection_set(
                    &mut frag_def.selection_set,
                    &fragment_map,
                    Some(&frag_def.type_condition),
                    &mut active_fragments,
                )?;
            }
        }
    }

    Ok(())
}

#[inline]
// active_fragments borrows names out of fragment_map ('f) instead of cloning strings -
// push/pop per expansion is then just a pointer compare over a handful of entries.
fn handle_selection_set<'a, 'f>(
    selection_set: &mut SelectionSet<'a, String>,
    fragment_map: &'f HashMap<String, FragmentDefinition<'a, String>>,
    parent_type_condition: Option<&TypeCondition<'a, String>>,
    active_fragments: &mut Vec<&'f str>,
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
                    active_fragments,
                )?;
                new_items.push(Selection::Field(field));
            }
            Selection::FragmentSpread(spread) => {
                // walk the spread chain iteratively so a long acyclic chain (...F1 -> ...F2 -> ...)
                // can't blow the stack. we only recurse on a fragment body that has real content.
                // `spread` stays a borrow throughout, retargeting into `fragment_map` per link so
                // no FragmentSpread gets cloned while advancing.
                let mut spread = &spread;
                // each chain step follows exactly one spread into one fragment, so a single walk
                // can visit each fragment at most once before it must either terminate or revisit -
                // if chain_len exceeds the number of known fragments, a name must have repeated,
                // which means we're in a cycle.
                //
                // example - acyclic chain (chain_len reaches 2, fragment_map.len() == 3, ok):
                //   fragment A on T { ...B }  fragment B on T { ...C }  fragment C on T { field }
                //
                // example - self-cycle (chain_len reaches 2, fragment_map.len() == 1, err):
                //   fragment A on T { ...A }
                //
                // example - mutual cycle (chain_len reaches 3, fragment_map.len() == 2, err):
                //   fragment A on T { ...B }  fragment B on T { ...A }
                //
                // spreading the same fragment multiple times does not break the check because each
                // spread site starts its own independent walk with its own chain_len reset to 0 -
                // the counter is local to one chain traversal, not shared across the selection set:
                //   query { ...A ...A }  fragment A on T { field }  <- two walks, each chain_len=0
                let max_chain = fragment_map.len();
                let mut chain_len = 0usize;
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
                        if let [Selection::FragmentSpread(next)] =
                            def.selection_set.items.as_slice()
                        {
                            chain_len += 1;
                            if chain_len > max_chain {
                                return Err(NormalizationError::CyclicFragmentSpread {
                                    fragment_name: spread.fragment_name.clone(),
                                });
                            }
                            spread = next;
                            continue;
                        }
                    }
                    break def;
                };

                // guards every recursive expansion below (inline and wrap alike), not just the
                // bare-spread fast path above - a sibling field or a directive on the cycling
                // spread breaks out of that fast path, so this is the backstop that catches it.
                if active_fragments.contains(&fragment_def.name.as_str()) {
                    return Err(NormalizationError::CyclicFragmentSpread {
                        fragment_name: spread.fragment_name.clone(),
                    });
                }
                active_fragments.push(fragment_def.name.as_str());

                let result = if parent_type_condition == Some(&fragment_def.type_condition)
                    // `...Frag @include(...)` stores `@include` on the spread itself.
                    // In the code below, we inline the fragment's selections,
                    // so any directives would be lost.
                    && spread.directives.is_empty()
                {
                    // If the fragment's type condition matches the top type condition,
                    // we can inline its selections directly.
                    let mut inlined = fragment_def.selection_set.clone();
                    handle_selection_set(
                        &mut inlined,
                        fragment_map,
                        parent_type_condition,
                        active_fragments,
                    )
                    .map(|_| new_items.extend(inlined.items))
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
                        active_fragments,
                    )
                    .map(|_| new_items.push(Selection::InlineFragment(inline_fragment)))
                };

                active_fragments.pop();
                result?;
            }
            Selection::InlineFragment(mut inline_fragment) => {
                handle_selection_set(
                    &mut inline_fragment.selection_set,
                    fragment_map,
                    inline_fragment.type_condition.as_ref(),
                    active_fragments,
                )?;
                new_items.push(Selection::InlineFragment(inline_fragment));
            }
        }
    }
    selection_set.items = new_items;

    Ok(())
}
