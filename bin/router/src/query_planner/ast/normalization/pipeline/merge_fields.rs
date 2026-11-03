use graphql_tools::parser::query::{
    Definition, Field, Mutation, OperationDefinition, Query, Selection, SelectionSet, Subscription,
};

use crate::query_planner::ast::normalization::context::NormalizationContext;
use crate::query_planner::ast::normalization::error::NormalizationError;
use crate::query_planner::utils::ast::equal_directives_arr;

#[inline]
pub fn merge_fields(ctx: &mut NormalizationContext) -> Result<(), NormalizationError> {
    for definition in &mut ctx.document.definitions {
        match definition {
            Definition::Operation(op_def) => match op_def {
                OperationDefinition::SelectionSet(selection_set) => {
                    handle_selection_set(selection_set)?;
                }
                OperationDefinition::Query(Query { selection_set, .. }) => {
                    handle_selection_set(selection_set)?;
                }
                OperationDefinition::Mutation(Mutation { selection_set, .. }) => {
                    handle_selection_set(selection_set)?;
                }
                OperationDefinition::Subscription(Subscription { selection_set, .. }) => {
                    handle_selection_set(selection_set)?;
                }
            },
            Definition::Fragment(_) => {
                // no longer relevant at this point, every fragment spread was inlined and defs will be dropped
            }
        }
    }

    Ok(())
}

#[inline]
fn fields_equal<'a>(a: &Field<'a, String>, b: &Field<'a, String>) -> bool {
    if a.alias != b.alias {
        return false;
    }

    if a.name != b.name {
        return false;
    }

    if a.arguments != b.arguments {
        return false;
    }

    if !equal_directives_arr(&a.directives, &b.directives) {
        return false;
    }

    true
}

#[inline]
fn handle_selection_set<'a>(
    selection_set: &mut SelectionSet<'a, String>,
) -> Result<(), NormalizationError> {
    let old_items = std::mem::take(&mut selection_set.items);
    let mut new_items: Vec<Selection<'a, String>> = Vec::with_capacity(old_items.len());

    // Equal fields go into the first one before going deeper, so each subtree is merged once.
    // Merging after going deeper ran the whole merged subtree again for every repeat.
    for selection in old_items {
        match selection {
            Selection::Field(mut current_field) => {
                let existing_field = new_items.iter_mut().find_map(|selection| match selection {
                    Selection::Field(field) if fields_equal(&current_field, field) => Some(field),
                    _ => None,
                });
                match existing_field {
                    Some(existing_field) => existing_field
                        .selection_set
                        .items
                        .append(&mut current_field.selection_set.items),
                    None => new_items.push(Selection::Field(current_field)),
                }
            }
            Selection::InlineFragment(inline_fragment) => {
                new_items.push(Selection::InlineFragment(inline_fragment));
            }
            Selection::FragmentSpread(_) => {
                // should be resolved and inlined by that point
            }
        }
    }

    for selection in &mut new_items {
        match selection {
            Selection::Field(field) => handle_selection_set(&mut field.selection_set)?,
            Selection::InlineFragment(fragment) => {
                handle_selection_set(&mut fragment.selection_set)?
            }
            Selection::FragmentSpread(_) => {}
        }
    }
    selection_set.items = new_items;

    Ok(())
}
