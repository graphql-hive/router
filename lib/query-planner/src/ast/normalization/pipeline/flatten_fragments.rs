use std::collections::{HashMap, HashSet};

use graphql_parser::query::{
    Definition, InlineFragment, Mutation, OperationDefinition, Query, Selection, SelectionSet,
    Subscription, TypeCondition,
};

use crate::{
    ast::normalization::{
        context::NormalizationContext,
        error::NormalizationError,
        utils::{extract_type_condition, vec_to_hashset},
    },
    state::supergraph_state::{SupergraphDefinition, SupergraphObjectType, SupergraphState},
};

type PossibleTypesMap<'a> = HashMap<&'a str, HashSet<String>>;

pub fn flatten_fragments(ctx: &mut NormalizationContext) -> Result<(), NormalizationError> {
    let mut possible_types = PossibleTypesMap::new();
    let maybe_subgraph_name = ctx.subgraph_name.as_ref();

    for (type_name, type_def) in ctx.supergraph.definitions.iter().filter(|(_, def)| {
        if let Some(subgraph_name) = maybe_subgraph_name {
            def.is_defined_in_subgraph(subgraph_name.as_str())
        } else {
            true
        }
    }) {
        match type_def {
            SupergraphDefinition::Union(union_type) => {
                possible_types.insert(
                    type_name,
                    vec_to_hashset(
                        &union_type
                            .union_members
                            .iter()
                            .filter_map(|m| {
                                if let Some(subgraph_name) = maybe_subgraph_name {
                                    if &m.graph == *subgraph_name {
                                        return None;
                                    }
                                }
                                Some(m.member.clone())
                            })
                            .collect::<Vec<String>>(),
                    ),
                );
            }
            SupergraphDefinition::Interface(_) => {
                let mut object_types: HashSet<String> = HashSet::new();
                for (obj_type_name, obj_type_def) in
                    ctx.supergraph.definitions.iter().filter(|(_, def)| {
                        if let Some(subgraph_name) = maybe_subgraph_name {
                            def.is_defined_in_subgraph(subgraph_name.as_str())
                        } else {
                            true
                        }
                    })
                {
                    if let SupergraphDefinition::Object(object_type) = obj_type_def {
                        if object_type.join_implements.iter().any(|j| {
                            let belongs = match maybe_subgraph_name {
                                Some(subgraph_name) => &j.graph_id == *subgraph_name,
                                None => true,
                            };

                            belongs && &j.interface == type_name
                        }) {
                            object_types.insert(obj_type_name.to_string());
                        }
                    }
                }
                possible_types.insert(type_name, object_types);
            }
            _ => {}
        }
    }

    let query_type_name = ctx.query_type_name();
    let mutation_type_name = ctx.mutation_type_name();
    let subscription_type_name = ctx.subscription_type_name();

    for definition in &mut ctx.document.definitions {
        match definition {
            Definition::Operation(op_def) => match op_def {
                OperationDefinition::SelectionSet(selection_set) => {
                    handle_selection_set(
                        ctx.supergraph,
                        &possible_types,
                        ctx.supergraph
                            .definitions
                            .get(query_type_name)
                            .ok_or_else(|| NormalizationError::SchemaTypeNotFound {
                                type_name: "Query".to_string(),
                            })?,
                        selection_set,
                    )?;
                }
                OperationDefinition::Query(Query { selection_set, .. }) => {
                    handle_selection_set(
                        ctx.supergraph,
                        &possible_types,
                        ctx.supergraph
                            .definitions
                            .get(query_type_name)
                            .ok_or_else(|| NormalizationError::SchemaTypeNotFound {
                                type_name: "Query".to_string(),
                            })?,
                        selection_set,
                    )?;
                }
                OperationDefinition::Mutation(Mutation { selection_set, .. }) => {
                    handle_selection_set(
                        ctx.supergraph,
                        &possible_types,
                        ctx.supergraph
                            .definitions
                            .get(mutation_type_name)
                            .ok_or_else(|| NormalizationError::SchemaTypeNotFound {
                                type_name: "Mutation".to_string(),
                            })?,
                        selection_set,
                    )?;
                }
                OperationDefinition::Subscription(Subscription { selection_set, .. }) => {
                    handle_selection_set(
                        ctx.supergraph,
                        &possible_types,
                        ctx.supergraph
                            .definitions
                            .get(subscription_type_name)
                            .ok_or_else(|| NormalizationError::SchemaTypeNotFound {
                                type_name: "Subscription".to_string(),
                            })?,
                        selection_set,
                    )?;
                }
            },
            Definition::Fragment(_) => {
                // no longer relevant at this point, every fragment spread was inlined and defs will be dropped
            }
        }
    }

    Ok(())
}

fn handle_selection_set(
    state: &SupergraphState,
    possible_types: &PossibleTypesMap,
    type_def: &SupergraphDefinition,
    selection_set: &mut SelectionSet<'static, String>,
) -> Result<(), NormalizationError> {
    let old_items = std::mem::take(&mut selection_set.items);
    let mut new_items: Vec<Selection<'static, String>> = Vec::new();

    // Get a list of fields implemented by the interface object
    let interface_fields: Option<std::collections::HashSet<String>> = match type_def {
        SupergraphDefinition::Interface(interface_type) => {
            let interface_object_in_graphs = interface_type
                .join_type
                .iter()
                .filter_map(|jt| match jt.is_interface_object {
                    true => Some(&jt.graph_id),
                    false => None,
                })
                .collect::<Vec<&String>>();

            if !interface_object_in_graphs.is_empty() {
                Some(
                    interface_type
                        .fields
                        .iter()
                        .filter_map(|(name, field)| {
                            // if a field is contributed by the interface object,
                            // it will have a single @join__field with no arguments
                            if field.join_field.iter().all(|j| j.graph_id.is_none())
                                || field.join_field.iter().any(|jf| {
                                    !jf.external
                                        && jf.graph_id.as_ref().is_some_and(|g| {
                                            interface_object_in_graphs.contains(&g)
                                        })
                                })
                            {
                                Some(name.clone())
                            } else {
                                None
                            }
                        })
                        .collect(),
                )
            } else {
                None
            }
        }
        _ => None,
    };

    let possible_object_types = match type_def {
        SupergraphDefinition::Interface(interface_type) => {
            let object_type_names = possible_types
                .get(interface_type.name.as_str())
                .ok_or_else(|| NormalizationError::PossibleTypesNotFound {
                    type_name: interface_type.name.clone(),
                })?;

            let mut object_types: Vec<&SupergraphObjectType> =
                Vec::with_capacity(object_type_names.len());

            for name in object_type_names {
                match state.definitions.get(name) {
                    Some(SupergraphDefinition::Object(obj)) => {
                        object_types.push(obj);
                    }
                    _ => {
                        return Err(NormalizationError::SchemaTypeNotFound {
                            type_name: name.clone(),
                        });
                    }
                }
            }
            object_types.sort_by_key(|obj| &obj.name);
            Some(object_types)
        }
        _ => None,
    };

    for selection in old_items {
        match selection {
            Selection::Field(mut field) => {
                if field.name.starts_with("__") {
                    new_items.push(Selection::Field(field));
                    continue;
                }

                if let Some(object_types) = &possible_object_types {
                    let is_interface_field = interface_fields
                        .as_ref()
                        .map(|fields| fields.contains(&field.name))
                        .unwrap_or(false);

                    let should_type_expand = if is_interface_field {
                        false
                    } else {
                        object_types.iter().any(|obj_type| {
                            if let Some(obj_field) = obj_type.fields.get(&field.name) {
                                obj_field.join_field.iter().any(|jf| jf.external)
                            } else {
                                true
                            }
                        })
                    };

                    // A selection set may target an interface type.
                    // However, not all implementing object types may resolve all interface fields
                    // (some fields may be marked as external).
                    // Type expansion is the process of rewriting a selection on an
                    // interface type into multiple inline fragments, each targeting a concrete object type that
                    // implements the interface type.
                    // With type expansion, the Query Planner will have to find at least
                    // one resolvable query path to each field.
                    // Instead of looking for Interface.Field edge,
                    // the Query Planner will look for Object.Field.
                    if should_type_expand {
                        let mut fragments = Vec::with_capacity(object_types.len());
                        for obj_type in object_types {
                            let mut new_field = field.clone();
                            if !new_field.selection_set.items.is_empty() {
                                if let Some(field_def) = obj_type.fields.get(&new_field.name) {
                                    let inner_type_name = field_def.field_type.inner_type();
                                    let inner_type_def = state
                                        .definitions
                                        .get(inner_type_name)
                                        .ok_or_else(|| NormalizationError::SchemaTypeNotFound {
                                            type_name: inner_type_name.to_string(),
                                        })?;

                                    handle_selection_set(
                                        state,
                                        possible_types,
                                        inner_type_def,
                                        &mut new_field.selection_set,
                                    )?;
                                }
                            }

                            fragments.push(Selection::InlineFragment(InlineFragment {
                                type_condition: Some(TypeCondition::On(obj_type.name.clone())),
                                directives: vec![],
                                selection_set: SelectionSet {
                                    span: Default::default(),
                                    items: vec![Selection::Field(new_field)],
                                },
                                position: Default::default(),
                            }));
                        }

                        new_items.extend(fragments);
                        continue;
                    }
                }

                let has_selection_set = !field.selection_set.items.is_empty();

                if has_selection_set {
                    let field_definition = type_def.fields().get(&field.name).ok_or_else(|| {
                        NormalizationError::FieldNotFoundInType {
                            field_name: field.name.clone(),
                            type_name: type_def.name().to_string(),
                        }
                    })?;
                    let inner_type_name = field_definition.field_type.inner_type();
                    handle_selection_set(
                        state,
                        possible_types,
                        state.definitions.get(inner_type_name).ok_or_else(|| {
                            NormalizationError::SchemaTypeNotFound {
                                type_name: inner_type_name.to_string(),
                            }
                        })?,
                        &mut field.selection_set,
                    )?;
                }

                new_items.push(Selection::Field(field));
            }
            Selection::InlineFragment(mut current_fragment) => {
                if current_fragment
                    .type_condition
                    .as_ref()
                    .is_some_and(|t| extract_type_condition(t) != type_def.name())
                {
                    // Type condition is present and different from the parent's type.
                    let type_condition_name = current_fragment
                        .type_condition
                        .as_ref()
                        .map(extract_type_condition)
                        .expect("type condition should exist");

                    let type_condition_def = state
                        .definitions
                        .get(&type_condition_name)
                        .ok_or_else(|| NormalizationError::SchemaTypeNotFound {
                            type_name: type_condition_name.clone(),
                        })?;

                    match type_condition_def {
                        SupergraphDefinition::Interface(_) => {
                            // When `... on I1 { id }`,
                            // but the field's output type is not `I1`, but `I2`,
                            // then we look for possible types of `I2`
                            // and possible types of `I1`.
                            // Next, we produce a list of object types
                            // that are possible for `I1` and `I2` (intersection).
                            // Finally, we take fragment's selection set,
                            // and create an inline fragment for every object type.
                            //
                            // We do it, because the Query Planner knows only about
                            // types matching the field's output type (field move edge)
                            // and (in case of interfaces)
                            // object types implementing the interface (abstract move edges).
                            //
                            // QP won't be able to find fields from `I1` as it knows only about `I2`,
                            // and object types implementing `I2`.
                            let object_types_of_type_cond = possible_types
                                .get(type_condition_name.as_str())
                                .ok_or_else(|| NormalizationError::PossibleTypesNotFound {
                                    type_name: type_condition_name.to_string(),
                                })?;

                            let object_types_of_current_type = match type_def {
                                SupergraphDefinition::Union(_)
                                | SupergraphDefinition::Interface(_) => possible_types
                                    .get(type_def.name())
                                    .ok_or_else(|| NormalizationError::PossibleTypesNotFound {
                                        type_name: type_def.name().to_string(),
                                    })?,
                                // For object types, the only possible type is itself
                                _ => &vec_to_hashset(&[type_def.name().to_string()]),
                            };

                            let mut sorted_object_types: Vec<String> = object_types_of_type_cond
                                .intersection(object_types_of_current_type)
                                .cloned()
                                .collect();
                            sorted_object_types.sort();

                            for object_type_name_str in sorted_object_types {
                                let mut new_fragment = current_fragment.clone();
                                new_fragment.type_condition =
                                    Some(TypeCondition::On(object_type_name_str.clone()));

                                handle_selection_set(
                                    state,
                                    possible_types,
                                    state.definitions.get(&object_type_name_str).ok_or_else(
                                        || NormalizationError::SchemaTypeNotFound {
                                            type_name: object_type_name_str.to_string(),
                                        },
                                    )?,
                                    &mut new_fragment.selection_set,
                                )?;
                                new_items.push(Selection::InlineFragment(new_fragment));
                            }
                        }
                        _ => {
                            handle_selection_set(
                                state,
                                possible_types,
                                type_condition_def,
                                &mut current_fragment.selection_set,
                            )?;
                            new_items.push(Selection::InlineFragment(current_fragment));
                        }
                    }
                } else {
                    handle_selection_set(
                        state,
                        possible_types,
                        type_def,
                        &mut current_fragment.selection_set,
                    )?;
                    new_items.extend(current_fragment.selection_set.items);
                }
            }
            Selection::FragmentSpread(_) => {}
        }
    }
    selection_set.items = new_items;

    Ok(())
}
