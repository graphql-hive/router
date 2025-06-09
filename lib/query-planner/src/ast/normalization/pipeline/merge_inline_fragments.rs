use graphql_parser::query::{
    Definition, InlineFragment, Mutation, OperationDefinition, Query, Selection, SelectionSet,
    Subscription,
};

use crate::ast::normalization::{context::NormalizationContext, error::NormalizationError};

pub fn merge_inline_fragments(ctx: &mut NormalizationContext) -> Result<(), NormalizationError> {
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

fn handle_selection_set<'a>(
    selection_set: &mut SelectionSet<'a, String>,
) -> Result<(), NormalizationError> {
    let old_items = std::mem::take(&mut selection_set.items);
    let mut new_items: Vec<Selection<'a, String>> = Vec::new();

    for selection in old_items {
        match selection {
            Selection::Field(mut field) => {
                handle_selection_set(&mut field.selection_set)?;
                new_items.push(Selection::Field(field));
            }
            Selection::InlineFragment(mut current_fragment) => {
                handle_selection_set(&mut current_fragment.selection_set)?;

                let mut merged_into_existing = false;
                for new_item_selection in new_items.iter_mut() {
                    if let Selection::InlineFragment(ref mut existing_fragment) = new_item_selection
                    {
                        if inline_fragments_equal(&current_fragment, existing_fragment) {
                            existing_fragment
                                .selection_set
                                .items
                                .append(&mut current_fragment.selection_set.items);

                            handle_selection_set(&mut existing_fragment.selection_set)?;

                            merged_into_existing = true;
                            break;
                        }
                    }
                }

                if !merged_into_existing {
                    new_items.push(Selection::InlineFragment(current_fragment));
                }
            }
            Selection::FragmentSpread(_) => {
                // gone at this point
            }
        }
    }
    selection_set.items = new_items;

    Ok(())
}

fn inline_fragments_equal<'a>(
    a: &InlineFragment<'a, String>,
    b: &InlineFragment<'a, String>,
) -> bool {
    if a.type_condition != b.type_condition {
        return false;
    }

    if a.directives != b.directives {
        return false;
    }

    true
}
