use std::collections::{HashMap, HashSet};

use graphql_parser::{
    query::{
        Definition, Mutation, OperationDefinition, Query, Selection, SelectionSet, Subscription,
        TypeCondition,
    },
    schema::TypeDefinition,
};
use graphql_tools::ast::{
    AbstractTypeDefinitionExtension, FieldByNameExtension, SchemaDocumentExtension,
    TypeDefinitionExtension, TypeExtension,
};

use crate::ast::normalization::{
    context::NormalizationContext,
    error::NormalizationError,
    utils::{extract_type_condition, vec_to_hashset},
};

type PossibleTypesMap<'a> = HashMap<&'a str, HashSet<String>>;

pub fn flatten_fragments(ctx: &mut NormalizationContext) -> Result<(), NormalizationError> {
    let mut possible_types = PossibleTypesMap::new();

    for (type_name, type_def) in ctx.schema.type_map() {
        match type_def {
            TypeDefinition::Union(union_type) => {
                possible_types.insert(type_name, vec_to_hashset(&union_type.types));
            }
            TypeDefinition::Interface(interface_type) => {
                let mut object_types: HashSet<String> = HashSet::new();
                for (obj_type_name, obj_type_def) in ctx.schema.type_map() {
                    if let TypeDefinition::Object(object_type) = obj_type_def {
                        if interface_type.is_implemented_by(object_type) {
                            object_types.insert(obj_type_name.to_string());
                        }
                    }
                }
                possible_types.insert(type_name, object_types);
            }
            _ => {}
        }
    }

    for definition in &mut ctx.document.definitions {
        match definition {
            Definition::Operation(op_def) => match op_def {
                OperationDefinition::SelectionSet(selection_set) => {
                    handle_selection_set(
                        ctx.schema,
                        &possible_types,
                        ctx.schema
                            .type_by_name(
                                ctx.schema
                                    .schema_definition()
                                    .query
                                    .as_ref()
                                    .unwrap_or(&"Query".to_string()),
                            )
                            .ok_or_else(|| NormalizationError::SchemaTypeNotFound {
                                type_name: "Query".to_string(),
                            })?,
                        selection_set,
                    )?;
                }
                OperationDefinition::Query(Query { selection_set, .. }) => {
                    handle_selection_set(
                        ctx.schema,
                        &possible_types,
                        ctx.schema
                            .type_by_name(
                                ctx.schema
                                    .schema_definition()
                                    .query
                                    .as_ref()
                                    .unwrap_or(&"Query".to_string()),
                            )
                            .ok_or_else(|| NormalizationError::SchemaTypeNotFound {
                                type_name: "Query".to_string(),
                            })?,
                        selection_set,
                    )?;
                }
                OperationDefinition::Mutation(Mutation { selection_set, .. }) => {
                    handle_selection_set(
                        ctx.schema,
                        &possible_types,
                        ctx.schema
                            .type_by_name(
                                ctx.schema
                                    .schema_definition()
                                    .mutation
                                    .as_ref()
                                    .unwrap_or(&"Mutation".to_string()),
                            )
                            .ok_or_else(|| NormalizationError::SchemaTypeNotFound {
                                type_name: "Mutation".to_string(),
                            })?,
                        selection_set,
                    )?;
                }
                OperationDefinition::Subscription(Subscription { selection_set, .. }) => {
                    handle_selection_set(
                        ctx.schema,
                        &possible_types,
                        ctx.schema
                            .type_by_name(
                                ctx.schema
                                    .schema_definition()
                                    .subscription
                                    .as_ref()
                                    .unwrap_or(&"Subscription".to_string()),
                            )
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
    schema: &graphql_parser::schema::Document<'static, String>,
    possible_types: &PossibleTypesMap,
    type_def: &graphql_parser::schema::TypeDefinition<'static, String>,
    selection_set: &mut SelectionSet<'static, String>,
) -> Result<(), NormalizationError> {
    let old_items = std::mem::take(&mut selection_set.items);
    let mut new_items: Vec<Selection<'static, String>> = Vec::new();

    for selection in old_items {
        match selection {
            Selection::Field(mut field) => {
                let has_selection_set = !field.selection_set.items.is_empty();

                if has_selection_set {
                    let field_definition =
                        type_def.field_by_name(&field.name).ok_or_else(|| {
                            NormalizationError::FieldNotFoundInType {
                                field_name: field.name.clone(),
                                type_name: type_def.name().to_string(),
                            }
                        })?;
                    let inner_type_name = field_definition.field_type.inner_type();
                    handle_selection_set(
                        schema,
                        possible_types,
                        schema.type_by_name(inner_type_name).ok_or_else(|| {
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

                    let type_condition_def =
                        schema.type_by_name(&type_condition_name).ok_or_else(|| {
                            NormalizationError::SchemaTypeNotFound {
                                type_name: type_condition_name.clone(),
                            }
                        })?;

                    match type_condition_def {
                        TypeDefinition::Interface(_) => {
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
                                TypeDefinition::Union(_) | TypeDefinition::Interface(_) => {
                                    possible_types.get(type_def.name()).ok_or_else(|| {
                                        NormalizationError::PossibleTypesNotFound {
                                            type_name: type_def.name().to_string(),
                                        }
                                    })?
                                }
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
                                    schema,
                                    possible_types,
                                    schema.type_by_name(&object_type_name_str).ok_or_else(
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
                                schema,
                                possible_types,
                                type_condition_def,
                                &mut current_fragment.selection_set,
                            )?;
                            new_items.push(Selection::InlineFragment(current_fragment));
                        }
                    }
                } else {
                    handle_selection_set(
                        schema,
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
