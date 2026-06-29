use std::vec;

use graphql_tools::parser::query::{
    Definition, Field, InlineFragment, Mutation, OperationDefinition, Query, Selection,
    SelectionSet, Subscription, TypeCondition,
};

use crate::{
    ast::normalization::{context::NormalizationContext, error::NormalizationError},
    state::supergraph_state::{SupergraphDefinition, SupergraphState},
};

/// A selection set may target an interface type.
/// However, not all implementing object types may resolve all interface fields
/// (some fields may be marked as external).
/// Type expansion is the process of rewriting a selection on an
/// interface type into multiple inline fragments, each targeting a concrete object type that
/// implements the interface type.
/// With type expansion, the Query Planner will have to find at least
/// one resolvable query path to each field.
/// Instead of looking for Interface.Field edge,
/// the Query Planner will look for Object.Field.
#[inline]
pub fn type_expand(ctx: &mut NormalizationContext) -> Result<(), NormalizationError> {
    let query_type_name = ctx.query_type_name();
    let mutation_type_name = ctx.mutation_type_name();
    let subscription_type_name = ctx.subscription_type_name();

    for definition in &mut ctx.document.definitions {
        match definition {
            Definition::Operation(op_def) => match op_def {
                OperationDefinition::SelectionSet(selection_set) => {
                    let root =
                        ctx.supergraph
                            .definitions
                            .get(query_type_name)
                            .ok_or_else(|| NormalizationError::SchemaTypeNotFound {
                                type_name: "Query".to_string(),
                            })?;
                    handle_selection_set(
                        ctx.supergraph,
                        ctx.subgraph_name,
                        root,
                        selection_set,
                    )?;
                }
                OperationDefinition::Query(Query { selection_set, .. }) => {
                    let root =
                        ctx.supergraph
                            .definitions
                            .get(query_type_name)
                            .ok_or_else(|| NormalizationError::SchemaTypeNotFound {
                                type_name: "Query".to_string(),
                            })?;
                    handle_selection_set(
                        ctx.supergraph,
                        ctx.subgraph_name,
                        root,
                        selection_set,
                    )?;
                }
                OperationDefinition::Mutation(Mutation { selection_set, .. }) => {
                    let root = ctx
                        .supergraph
                        .definitions
                        .get(mutation_type_name)
                        .ok_or_else(|| NormalizationError::SchemaTypeNotFound {
                            type_name: "Mutation".to_string(),
                        })?;
                    handle_selection_set(
                        ctx.supergraph,
                        ctx.subgraph_name,
                        root,
                        selection_set,
                    )?;
                }
                OperationDefinition::Subscription(Subscription { selection_set, .. }) => {
                    let root = ctx
                        .supergraph
                        .definitions
                        .get(subscription_type_name)
                        .ok_or_else(|| NormalizationError::SchemaTypeNotFound {
                            type_name: "Subscription".to_string(),
                        })?;
                    handle_selection_set(
                        ctx.supergraph,
                        ctx.subgraph_name,
                        root,
                        selection_set,
                    )?;
                }
            },
            Definition::Fragment(_) => {}
        }
    }

    Ok(())
}

#[inline]
fn handle_selection_set<'schema, 'sel>(
    state: &'schema SupergraphState,
    subgraph_name: Option<&'schema str>,
    type_def: &'schema SupergraphDefinition,
    selection_set: &mut SelectionSet<'sel, String>,
) -> Result<(), NormalizationError> {
    let old_items = std::mem::take(&mut selection_set.items);
    let mut new_items = Vec::with_capacity(old_items.len());

    for selection in old_items {
        match selection {
            Selection::Field(mut field) => {
                // Don't try to look up introspection fields in the schema
                // Just keep the selection set as-is
                if field.name.starts_with("__") {
                    new_items.push(Selection::Field(field));
                    continue;
                }

                let field_def = type_def.fields().get(&field.name).ok_or_else(|| {
                    NormalizationError::FieldNotFoundInType {
                        field_name: field.name.clone(),
                        type_name: type_def.name().to_string(),
                    }
                })?;

                if handle_type_expansion_candidate(
                    state,
                    subgraph_name,
                    type_def,
                    &field,
                    &mut new_items,
                )? {
                    continue;
                }

                if !field.selection_set.items.is_empty() {
                    let inner_type_name = field_def.field_type.inner_type();
                    let inner_type_def =
                        state.definitions.get(inner_type_name).ok_or_else(|| {
                            NormalizationError::SchemaTypeNotFound {
                                type_name: inner_type_name.to_string(),
                            }
                        })?;
                    handle_selection_set(
                        state,
                        subgraph_name,
                        inner_type_def,
                        &mut field.selection_set,
                    )?;
                }
                new_items.push(Selection::Field(field));
            }
            Selection::InlineFragment(mut frag) => {
                if let Some(type_cond) = &frag.type_condition {
                    let TypeCondition::On(type_name) = type_cond;
                    if let Some(type_def) = state.definitions.get(type_name) {
                        handle_selection_set(
                            state,
                            subgraph_name,
                            type_def,
                            &mut frag.selection_set,
                        )?;
                    }
                } else {
                    handle_selection_set(
                        state,
                        subgraph_name,
                        type_def,
                        &mut frag.selection_set,
                    )?;
                }
                new_items.push(Selection::InlineFragment(frag));
            }
            other => {
                new_items.push(other);
            }
        }
    }

    selection_set.items = new_items;
    Ok(())
}
type ShouldContinue = bool;

#[inline]
fn handle_type_expansion_candidate<'schema, 'sel>(
    state: &'schema SupergraphState,
    subgraph_name: Option<&'schema str>,
    type_def: &'schema SupergraphDefinition,
    field: &Field<'sel, String>,
    new_items: &mut Vec<Selection<'sel, String>>,
) -> Result<ShouldContinue, NormalizationError> {
    let interface_type = match type_def {
        SupergraphDefinition::Interface(interface_type) => Some(interface_type),
        _ => None,
    };

    if interface_type.is_none() {
        return Ok(false);
    }
    let interface_type = interface_type.unwrap(); // safe due to previous check

    let field_def = interface_type.fields.get(&field.name);
    if field_def.is_none() {
        return Ok(false);
    }

    let field_def = field_def.unwrap();

    // An interface may be defined in multiple subgraphs.
    // If a field on that interface is not resolvable in all of those subgraphs,
    // we must expand to concrete types to ensure we can find a resolvable query path.
    let subgraphs_with_interface = interface_type
        .join_type
        .iter()
        // if one of the interfaces is @interfaceObject,
        // ignore it
        .filter(|jt| !jt.is_interface_object)
        .count();

    if subgraphs_with_interface <= 1 {
        // No need to expand if interface is in at most one subgraph.
        return Ok(false);
    }

    if field_def.join_field.is_empty() {
        // The field is available everywhere the interface is defined if there's no join info
        return Ok(false);
    }

    // Check if the field is contributed by the interface object,
    // which means it's available in all subgraphs that define the interface.
    let interface_object_in_graphs: Vec<_> = interface_type
        .join_type
        .iter()
        .filter_map(|jt| {
            if jt.is_interface_object {
                Some(&jt.graph_id)
            } else {
                None
            }
        })
        .collect();

    let is_interface_object_field = field_def.join_field.iter().all(|j| j.graph_id.is_none())
        || field_def.join_field.iter().any(|jf| {
            !jf.external
                && jf
                    .graph_id
                    .as_ref()
                    .is_some_and(|g| interface_object_in_graphs.contains(&g))
        });

    if is_interface_object_field {
        return Ok(false);
    }

    // All subgraphs where this field is resolvable
    let subgraphs_resolving_field = field_def
        .join_field
        .iter()
        .filter(|jf| !jf.external && jf.graph_id.is_some())
        .count();

    if subgraphs_resolving_field >= subgraphs_with_interface {
        // All subgraphs are resolving the field
        return Ok(false);
    }

    let possible_object_types = state
        .abstract_possible_types(interface_type.name.as_str(), subgraph_name)
        .ok_or_else(|| NormalizationError::PossibleTypesNotFound {
            type_name: interface_type.name.clone(),
        })?;

    let should_expand = possible_object_types.iter().any(|obj| {
        let Some(SupergraphDefinition::Object(obj_def)) = state.definitions.get(obj.as_str()) else {
            return true;
        };
        // Expand if any object type implementing the interface:
        // 1. Does not have the field.
        // 2. Has the field, but it's marked as external or is overridden.
        match obj_def.fields.get(&field.name) {
            None => true,
            Some(obj_field) => obj_field.join_field.iter().any(|jf| {
                jf.external
                    || jf.used_overridden
                    || jf
                        .override_value
                        .as_ref()
                        .is_some_and(|name| state.subgraph_exists_by_name(name))
            }),
        }
    });

    if !should_expand {
        return Ok(false);
    }

    // Sort object_types by name for deterministic fragment order
    let mut sorted_object_types: Vec<_> = possible_object_types.iter().collect();
    sorted_object_types.sort_unstable();

    let mut fragments = Vec::with_capacity(sorted_object_types.len());
    for obj in sorted_object_types {
        let mut new_field = field.clone();
        let obj_def = state.definitions.get(obj.as_str()).ok_or_else(|| {
            NormalizationError::SchemaTypeNotFound {
                type_name: obj.to_string(),
            }
        })?;
        let SupergraphDefinition::Object(obj) = obj_def else {
            return Err(NormalizationError::SchemaTypeNotFound {
                type_name: obj_def.name().to_string(),
            });
        };
        if !new_field.selection_set.items.is_empty() {
            if let Some(obj_field) = obj.fields.get(&new_field.name) {
                let inner_type_name = obj_field.field_type.inner_type();
                let inner_type_def = state.definitions.get(inner_type_name).ok_or_else(|| {
                    NormalizationError::SchemaTypeNotFound {
                        type_name: inner_type_name.to_string(),
                    }
                })?;
                handle_selection_set(
                    state,
                    subgraph_name,
                    inner_type_def,
                    &mut new_field.selection_set,
                )?;
            }
        }
        fragments.push(Selection::InlineFragment(InlineFragment {
            type_condition: Some(TypeCondition::On(obj.name.clone())),
            directives: field.directives.clone(),
            selection_set: SelectionSet {
                span: Default::default(),
                items: vec![Selection::Field(new_field)],
            },
            position: Default::default(),
        }));
    }
    new_items.extend(fragments);

    Ok(true)
}
