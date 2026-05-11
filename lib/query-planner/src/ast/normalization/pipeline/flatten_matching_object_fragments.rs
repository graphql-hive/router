use graphql_tools::parser::query::{
    Definition, Field, Mutation, OperationDefinition, Query, Selection, SelectionSet, Subscription,
};

use crate::ast::normalization::context::NormalizationContext;
use crate::ast::normalization::error::NormalizationError;
use crate::ast::normalization::pipeline::matching_object_fragment::can_flatten_matching_object_fragment;
use crate::state::supergraph_state::{SupergraphDefinition, SupergraphState};

#[inline]
pub fn flatten_matching_object_fragments(
    ctx: &mut NormalizationContext,
) -> Result<(), NormalizationError> {
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
    for selection in &mut selection_set.items {
        match selection {
            Selection::Field(field) => process_field(state, parent_type_def, field)?,
            Selection::InlineFragment(fragment) => {
                let child_parent_type = fragment
                    .type_condition
                    .as_ref()
                    .and_then(|type_condition| {
                        let graphql_tools::parser::query::TypeCondition::On(type_name) =
                            type_condition;
                        state.definitions.get(type_name)
                    })
                    .unwrap_or(parent_type_def);

                handle_selection_set(state, &mut fragment.selection_set, child_parent_type)?;
            }
            Selection::FragmentSpread(_) => {
                // gone at this point
            }
        }
    }

    Ok(())
}

#[inline]
fn process_field<'a>(
    state: &SupergraphState,
    parent_type_def: &SupergraphDefinition,
    field: &mut Field<'a, String>,
) -> Result<(), NormalizationError> {
    if field.name.starts_with("__") || field.selection_set.items.is_empty() {
        return Ok(());
    }

    let field_type_name = parent_type_def
        .fields()
        .get(&field.name)
        .ok_or_else(|| NormalizationError::FieldNotFoundInType {
            field_name: field.name.clone(),
            type_name: parent_type_def.name().to_string(),
        })?
        .field_type
        .inner_type();
    let field_type_def = state.definitions.get(field_type_name).ok_or_else(|| {
        NormalizationError::SchemaTypeNotFound {
            type_name: field_type_name.to_string(),
        }
    })?;

    handle_selection_set(state, &mut field.selection_set, field_type_def)?;

    if matches!(field_type_def, SupergraphDefinition::Object(_)) {
        flatten_field_selection_set(&mut field.selection_set, field_type_def.name());
    }

    Ok(())
}

#[inline]
fn flatten_field_selection_set<'a>(
    selection_set: &mut SelectionSet<'a, String>,
    object_type_name: &str,
) {
    let old_items = std::mem::take(&mut selection_set.items);
    let mut new_items = Vec::with_capacity(old_items.len());

    for selection in old_items {
        match selection {
            Selection::InlineFragment(mut fragment)
                if can_flatten_matching_object_fragment(&fragment, object_type_name) =>
            {
                new_items.append(&mut fragment.selection_set.items);
            }
            other => new_items.push(other),
        }
    }

    selection_set.items = new_items;
}
