use graphql_parser::query::{
    Definition, Field, Mutation, OperationDefinition, Query, Selection, SelectionSet, Subscription,
};

use crate::ast::normalization::{context::NormalizationContext, error::NormalizationError};

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

    if a.directives != b.directives {
        return false;
    }

    true
}

fn handle_selection_set<'a>(
    selection_set: &mut SelectionSet<'a, String>,
) -> Result<(), NormalizationError> {
    let old_items = std::mem::take(&mut selection_set.items);
    let mut new_items: Vec<Selection<'a, String>> = Vec::new();

    for selection in old_items {
        match selection {
            Selection::Field(mut current_field) => {
                handle_selection_set(&mut current_field.selection_set)?;

                let mut merged_into_existing = false;
                for new_item_selection in new_items.iter_mut() {
                    if let Selection::Field(ref mut existing_field) = new_item_selection {
                        if fields_equal(&current_field, existing_field) {
                            existing_field
                                .selection_set
                                .items
                                .append(&mut current_field.selection_set.items);

                            handle_selection_set(&mut existing_field.selection_set)?;
                            merged_into_existing = true;
                            break;
                        }
                    }
                }

                if !merged_into_existing {
                    new_items.push(Selection::Field(current_field));
                }
            }
            Selection::InlineFragment(mut inline_fragment) => {
                handle_selection_set(&mut inline_fragment.selection_set)?;
                new_items.push(Selection::InlineFragment(inline_fragment));
            }
            Selection::FragmentSpread(_) => {
                // should be resolved and inlined by that point
            }
        }
    }
    selection_set.items = new_items;

    Ok(())
}
