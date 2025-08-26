use graphql_parser::query::{
    Definition, Field, FragmentDefinition, Mutation, OperationDefinition, Query, Selection,
    SelectionSet, Subscription,
};

use crate::ast::normalization::context::NormalizationContext;
use crate::ast::normalization::error::NormalizationError;
use crate::utils::ast::equal_directives_arr;

#[inline]
pub fn drop_duplicated_fields(ctx: &mut NormalizationContext) -> Result<(), NormalizationError> {
    for def in &mut ctx.document.definitions {
        match def {
            Definition::Operation(op) => match op {
                OperationDefinition::Query(Query { selection_set, .. }) => {
                    handle_selection_set(selection_set)?;
                }
                OperationDefinition::Mutation(Mutation { selection_set, .. }) => {
                    handle_selection_set(selection_set)?;
                }
                OperationDefinition::Subscription(Subscription { selection_set, .. }) => {
                    handle_selection_set(selection_set)?;
                }
                OperationDefinition::SelectionSet(s) => {
                    handle_selection_set(s)?;
                }
            },
            Definition::Fragment(FragmentDefinition { selection_set, .. }) => {
                handle_selection_set(selection_set)?;
            }
        }
    }

    Ok(())
}

#[inline]
fn are_fields_shallow_equal<'a>(a: &Field<'a, String>, b: &Field<'a, String>) -> bool {
    if a.name != b.name {
        return false;
    }

    if a.alias != b.alias {
        return false;
    }

    if a.arguments != b.arguments {
        return false;
    }

    if !equal_directives_arr(&a.directives, &b.directives) {
        return false;
    }

    a.selection_set.items.is_empty() && b.selection_set.items.is_empty()
}

#[inline]
fn handle_selection_set<'a>(
    selection_set: &mut SelectionSet<'a, String>,
) -> Result<(), NormalizationError> {
    if selection_set.items.len() < 2 {
        return Ok(());
    }

    let original_items = std::mem::take(&mut selection_set.items);
    let mut new_items = Vec::with_capacity(original_items.len());

    for mut current_item_candidate in original_items {
        let mut should_add = true;

        match current_item_candidate {
            Selection::Field(ref field_to_check) => {
                let mut is_duplicate = false;
                for processed_item in &new_items {
                    if let Selection::Field(ref kept_field) = processed_item {
                        if are_fields_shallow_equal(field_to_check, kept_field) {
                            is_duplicate = true;
                            break;
                        }
                    }
                }

                if is_duplicate {
                    should_add = false;
                }
            }
            Selection::InlineFragment(ref mut inline_frag) => {
                handle_selection_set(&mut inline_frag.selection_set)?;
            }
            Selection::FragmentSpread(_) => {}
        }

        if should_add {
            new_items.push(current_item_candidate);
        }
    }

    selection_set.items = new_items;

    Ok(())
}
