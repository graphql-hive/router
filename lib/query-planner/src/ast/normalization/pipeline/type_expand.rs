use std::collections::HashSet;

use graphql_parser::query::{
    Definition, InlineFragment, Mutation, OperationDefinition, Query, Selection, SelectionSet,
    Subscription, TypeCondition,
};

use crate::{
    ast::normalization::pipeline::flatten_fragments::PossibleTypesMap,
    ast::normalization::utils::vec_to_hashset,
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
pub fn type_expand(ctx: &mut NormalizationContext) -> Result<(), NormalizationError> {
    let mut possible_types = PossibleTypesMap::new();
    let maybe_subgraph_name = ctx.subgraph_name.as_ref();

    // Build possible_types map (same as flatten_fragments)
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
                            .map(|m| m.member.clone())
                            .collect::<Vec<String>>(),
                    ),
                );
            }
            SupergraphDefinition::Interface(_) => {
                let mut object_types: HashSet<String> = HashSet::new();
                for (obj_type_name, obj_type_def) in ctx.supergraph.definitions.iter() {
                    if let SupergraphDefinition::Object(object_type) = obj_type_def {
                        if object_type
                            .join_implements
                            .iter()
                            .any(|j| &j.interface == type_name)
                        {
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
                    let root =
                        ctx.supergraph
                            .definitions
                            .get(query_type_name)
                            .ok_or_else(|| NormalizationError::SchemaTypeNotFound {
                                type_name: "Query".to_string(),
                            })?;
                    handle_selection_set(ctx.supergraph, &possible_types, root, selection_set)?;
                }
                OperationDefinition::Query(Query { selection_set, .. }) => {
                    let root =
                        ctx.supergraph
                            .definitions
                            .get(query_type_name)
                            .ok_or_else(|| NormalizationError::SchemaTypeNotFound {
                                type_name: "Query".to_string(),
                            })?;
                    handle_selection_set(ctx.supergraph, &possible_types, root, selection_set)?;
                }
                OperationDefinition::Mutation(Mutation { selection_set, .. }) => {
                    let root = ctx
                        .supergraph
                        .definitions
                        .get(mutation_type_name)
                        .ok_or_else(|| NormalizationError::SchemaTypeNotFound {
                            type_name: "Mutation".to_string(),
                        })?;
                    handle_selection_set(ctx.supergraph, &possible_types, root, selection_set)?;
                }
                OperationDefinition::Subscription(Subscription { selection_set, .. }) => {
                    let root = ctx
                        .supergraph
                        .definitions
                        .get(subscription_type_name)
                        .ok_or_else(|| NormalizationError::SchemaTypeNotFound {
                            type_name: "Subscription".to_string(),
                        })?;
                    handle_selection_set(ctx.supergraph, &possible_types, root, selection_set)?;
                }
            },
            Definition::Fragment(_) => {}
        }
    }

    Ok(())
}

fn handle_selection_set(
    state: &SupergraphState,
    possible_types: &PossibleTypesMap,
    type_def: &SupergraphDefinition,
    selection_set: &mut SelectionSet<String>,
) -> Result<(), NormalizationError> {
    let old_items = std::mem::take(&mut selection_set.items);
    let mut new_items = Vec::with_capacity(old_items.len());

    // Only perform type expansion if the current type is abstract.
    let possible_object_types = match type_def {
        SupergraphDefinition::Interface(interface_type) => {
            let object_names = possible_types
                .get(interface_type.name.as_str())
                .ok_or_else(|| NormalizationError::PossibleTypesNotFound {
                    type_name: interface_type.name.clone(),
                })?;
            let mut objects = Vec::with_capacity(object_names.len());
            for name in object_names {
                if let Some(SupergraphDefinition::Object(obj)) = state.definitions.get(name) {
                    objects.push(obj);
                } else {
                    return Err(NormalizationError::SchemaTypeNotFound {
                        type_name: name.clone(),
                    });
                }
            }
            Some(objects)
        }
        SupergraphDefinition::Union(union_type) => {
            let mut objects = Vec::new();
            for member in &union_type.union_members {
                if let Some(SupergraphDefinition::Object(obj)) =
                    state.definitions.get(&member.member)
                {
                    objects.push(obj);
                } else {
                    return Err(NormalizationError::SchemaTypeNotFound {
                        type_name: member.member.clone(),
                    });
                }
            }
            Some(objects)
        }
        _ => None,
    };

    for selection in old_items {
        match selection {
            Selection::Field(mut field) => {
                // Recurse into sub-selection sets
                if !field.selection_set.items.is_empty() {
                    if field.name.starts_with("__") {
                        // Don't try to look up introspection fields in the schema
                        // Just keep the selection set as-is
                    } else {
                        let inner_type_name = type_def
                            .fields()
                            .get(&field.name)
                            .ok_or_else(|| NormalizationError::FieldNotFoundInType {
                                field_name: field.name.clone(),
                                type_name: type_def.name().to_string(),
                            })?
                            .field_type
                            .inner_type();
                        let inner_type_def =
                            state.definitions.get(inner_type_name).ok_or_else(|| {
                                NormalizationError::SchemaTypeNotFound {
                                    type_name: inner_type_name.to_string(),
                                }
                            })?;
                        handle_selection_set(
                            state,
                            possible_types,
                            inner_type_def,
                            &mut field.selection_set,
                        )?;
                    }
                }

                if let Some(object_types) = &possible_object_types {
                    if field.name.starts_with("__") {
                        new_items.push(Selection::Field(field));
                        continue;
                    }

                    let mut interface_needs_expansion_due_to_subgraphs = false;
                    if let SupergraphDefinition::Interface(interface_type) = type_def {
                        let mut subgraphs_with_interface = std::collections::HashSet::new();
                        let mut subgraphs_resolving_field = std::collections::HashSet::new();

                        for jt in &interface_type.join_type {
                            if !jt.is_interface_object {
                                subgraphs_with_interface.insert(&jt.graph_id);
                            }
                        }

                        if let Some(field_def) = interface_type.fields.get(&field.name) {
                            // Check if the field is contributed by the interface object
                            let interface_object_in_graphs = interface_type
                                .join_type
                                .iter()
                                .filter_map(|jt| match jt.is_interface_object {
                                    true => Some(&jt.graph_id),
                                    false => None,
                                })
                                .collect::<Vec<&String>>();

                            let is_interface_object_field =
                                field_def.join_field.iter().all(|j| j.graph_id.is_none())
                                    || field_def.join_field.iter().any(|jf| {
                                        !jf.external
                                            && jf.graph_id.as_ref().is_some_and(|g| {
                                                interface_object_in_graphs.contains(&g)
                                            })
                                    });

                            if field_def.join_field.is_empty() {
                                // No join__field: field is available everywhere the interface is defined
                                interface_needs_expansion_due_to_subgraphs = false;
                            } else if !is_interface_object_field {
                                for jf in &field_def.join_field {
                                    if !jf.external {
                                        if let Some(graph_id) = &jf.graph_id {
                                            subgraphs_resolving_field.insert(graph_id);
                                        }
                                    }
                                }
                                if subgraphs_with_interface.len() > 1
                                    && subgraphs_resolving_field.len()
                                        < subgraphs_with_interface.len()
                                {
                                    interface_needs_expansion_due_to_subgraphs = true;
                                }
                            }
                        }
                    }

                    let should_expand = interface_needs_expansion_due_to_subgraphs
                        || object_types.iter().any(|obj| {
                            obj.fields.get(&field.name).is_none()
                                || obj
                                    .fields
                                    .get(&field.name)
                                    .map(|obj_field| {
                                        obj_field.join_field.iter().any(|jf| jf.external)
                                    })
                                    .unwrap_or(false)
                        });

                    if should_expand {
                        // Sort object_types by name for deterministic fragment order
                        let mut sorted_object_types: Vec<_> = object_types.iter().collect();
                        sorted_object_types.sort_by(|a, b| a.name.cmp(&b.name));

                        let mut fragments = Vec::with_capacity(sorted_object_types.len());
                        for obj in sorted_object_types {
                            let mut new_field = field.clone();
                            if !new_field.selection_set.items.is_empty() {
                                if let Some(obj_field) = obj.fields.get(&new_field.name) {
                                    let inner_type_name = obj_field.field_type.inner_type();
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
                                type_condition: Some(TypeCondition::On(obj.name.clone())),
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
                new_items.push(Selection::Field(field));
            }
            Selection::InlineFragment(mut frag) => {
                // Recurse into nested fragments
                if let Some(ref type_cond) = frag.type_condition {
                    let type_name = match type_cond {
                        TypeCondition::On(name) => name,
                    };
                    if let Some(type_def) = state.definitions.get(type_name) {
                        handle_selection_set(
                            state,
                            possible_types,
                            type_def,
                            &mut frag.selection_set,
                        )?;
                    }
                } else {
                    handle_selection_set(state, possible_types, type_def, &mut frag.selection_set)?;
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
