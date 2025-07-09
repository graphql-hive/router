use graphql_parser::query::{
    Definition, Directive, FragmentDefinition, Mutation, OperationDefinition, Query, Selection,
    SelectionSet, Subscription, Value,
};

use crate::ast::normalization::{context::NormalizationContext, error::NormalizationError};

pub fn drop_skipped_fields(ctx: &mut NormalizationContext) -> Result<(), NormalizationError> {
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

fn handle_selection_set<'a>(
    selection_set: &mut SelectionSet<'a, String>,
) -> Result<(), NormalizationError> {
    if selection_set.items.is_empty() {
        return Ok(());
    }

    let original_items = std::mem::take(&mut selection_set.items);
    let mut new_items = Vec::with_capacity(original_items.len());

    for mut current_item_candidate in original_items {
        let mut should_add = true;

        match current_item_candidate {
            Selection::Field(ref mut field) => {
                should_add = should_keep(&field.directives);
                if should_add {
                    handle_selection_set(&mut field.selection_set)?;
                }
            }
            Selection::InlineFragment(ref mut inline_frag) => {
                should_add = should_keep(&inline_frag.directives);
                if should_add {
                    handle_selection_set(&mut inline_frag.selection_set)?;
                }
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

fn should_keep(directives: &Vec<Directive<'_, String>>) -> bool {
    if extract_condition_directive("skip", directives).is_some_and(|skip| skip) {
        return false;
    }

    if extract_condition_directive("include", directives).is_some_and(|include| !include) {
        return false;
    }

    true
}

fn extract_condition_directive(
    directive_name: &str,
    directives: &Vec<Directive<'_, String>>,
) -> Option<bool> {
    directives.iter().find_map(|d| {
        if d.name != directive_name {
            return None;
        }
        d.arguments.iter().find_map(|(name, value)| {
            if name != "if" {
                return None;
            }

            match value {
                Value::Boolean(b) => Some(*b),
                _ => None,
            }
        })
    })
}
