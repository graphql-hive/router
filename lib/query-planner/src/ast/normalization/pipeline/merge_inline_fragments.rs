use graphql_tools::parser::query::{
    Definition, InlineFragment, Mutation, OperationDefinition, Query, Selection, SelectionSet,
    Subscription,
};

use crate::ast::normalization::context::NormalizationContext;
use crate::ast::normalization::error::NormalizationError;
use crate::state::supergraph_state::{SupergraphDefinition, SupergraphState};
use crate::utils::ast::equal_directives_arr;

#[inline]
pub fn merge_inline_fragments(ctx: &mut NormalizationContext) -> Result<(), NormalizationError> {
    let supergraph = ctx.supergraph;
    let query_type_name = ctx.query_type_name().to_string();
    let mutation_type_name = ctx.mutation_type_name().to_string();
    let subscription_type_name = ctx.subscription_type_name().to_string();

    for definition in &mut ctx.document.definitions {
        match definition {
            Definition::Operation(op_def) => match op_def {
                OperationDefinition::SelectionSet(selection_set) => {
                    let root_def = supergraph
                        .definitions
                        .get(query_type_name.as_str())
                        .ok_or_else(|| NormalizationError::SchemaTypeNotFound {
                            type_name: query_type_name.clone(),
                        })?;
                    handle_selection_set(supergraph, selection_set, root_def)?;
                }
                OperationDefinition::Query(Query { selection_set, .. }) => {
                    let root_def = supergraph
                        .definitions
                        .get(query_type_name.as_str())
                        .ok_or_else(|| NormalizationError::SchemaTypeNotFound {
                            type_name: query_type_name.clone(),
                        })?;
                    handle_selection_set(supergraph, selection_set, root_def)?;
                }
                OperationDefinition::Mutation(Mutation { selection_set, .. }) => {
                    let root_def = supergraph
                        .definitions
                        .get(mutation_type_name.as_str())
                        .ok_or_else(|| NormalizationError::SchemaTypeNotFound {
                            type_name: mutation_type_name.clone(),
                        })?;
                    handle_selection_set(supergraph, selection_set, root_def)?;
                }
                OperationDefinition::Subscription(Subscription { selection_set, .. }) => {
                    let root_def = supergraph
                        .definitions
                        .get(subscription_type_name.as_str())
                        .ok_or_else(|| NormalizationError::SchemaTypeNotFound {
                            type_name: subscription_type_name.clone(),
                        })?;
                    handle_selection_set(supergraph, selection_set, root_def)?;
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
fn handle_selection_set<'a>(
    state: &SupergraphState,
    selection_set: &mut SelectionSet<'a, String>,
    parent_type_def: &SupergraphDefinition,
) -> Result<(), NormalizationError> {
    let old_items = std::mem::take(&mut selection_set.items);
    let mut new_items: Vec<Selection<'a, String>> = Vec::new();

    for selection in old_items {
        match selection {
            Selection::Field(mut field) => {
                if field.name.starts_with("__") {
                    new_items.push(Selection::Field(field));
                    continue;
                }

                if !field.selection_set.items.is_empty() {
                    let field_type_name = parent_type_def
                        .fields()
                        .get(&field.name)
                        .ok_or_else(|| NormalizationError::FieldNotFoundInType {
                            field_name: field.name.clone(),
                            type_name: parent_type_def.name().to_string(),
                        })?
                        .field_type
                        .inner_type();
                    let field_type_def =
                        state.definitions.get(field_type_name).ok_or_else(|| {
                            NormalizationError::SchemaTypeNotFound {
                                type_name: field_type_name.to_string(),
                            }
                        })?;

                    handle_selection_set(state, &mut field.selection_set, field_type_def)?;
                }
                new_items.push(Selection::Field(field));
            }
            Selection::InlineFragment(mut current_fragment) => {
                let child_parent_type = current_fragment
                    .type_condition
                    .as_ref()
                    .and_then(|type_condition| {
                        let graphql_tools::parser::query::TypeCondition::On(type_name) =
                            type_condition;
                        state.definitions.get(type_name)
                    })
                    .unwrap_or(parent_type_def);

                handle_selection_set(
                    state,
                    &mut current_fragment.selection_set,
                    child_parent_type,
                )?;
                flatten_redundant_nested_inline_fragments(&mut current_fragment);

                let mut merged_into_existing = false;
                for new_item_selection in new_items.iter_mut() {
                    if let Selection::InlineFragment(ref mut existing_fragment) = new_item_selection
                    {
                        if inline_fragments_equal(&current_fragment, existing_fragment) {
                            existing_fragment
                                .selection_set
                                .items
                                .append(&mut current_fragment.selection_set.items);

                            handle_selection_set(
                                state,
                                &mut existing_fragment.selection_set,
                                child_parent_type,
                            )?;

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

#[inline]
fn inline_fragments_equal<'a>(
    a: &InlineFragment<'a, String>,
    b: &InlineFragment<'a, String>,
) -> bool {
    if a.type_condition != b.type_condition {
        return false;
    }

    if !equal_directives_arr(&a.directives, &b.directives) {
        return false;
    }

    true
}

#[inline]
fn flatten_redundant_nested_inline_fragments<'a>(parent: &mut InlineFragment<'a, String>) {
    let old_items = std::mem::take(&mut parent.selection_set.items);
    let mut new_items = Vec::with_capacity(old_items.len());

    for selection in old_items {
        match selection {
            Selection::InlineFragment(mut child) if can_flatten_into_parent(parent, &child) => {
                new_items.append(&mut child.selection_set.items);
            }
            other => new_items.push(other),
        }
    }

    parent.selection_set.items = new_items;
}

#[inline]
fn can_flatten_into_parent<'a>(
    parent: &InlineFragment<'a, String>,
    child: &InlineFragment<'a, String>,
) -> bool {
    child.type_condition == parent.type_condition
        && (child.directives.is_empty()
            || equal_directives_arr(&child.directives, &parent.directives))
}
