use graphql_parser::query::{
    Definition, Field, InlineFragment, Mutation, OperationDefinition, Query, Selection,
    SelectionSet, Subscription, TypeCondition,
};
use hashbrown::{HashMap, HashSet};

use crate::{
    ast::normalization::{
        context::NormalizationContext, error::NormalizationError, utils::extract_type_condition,
    },
    state::supergraph_state::{SupergraphDefinition, SupergraphState},
};

pub type PossibleTypesMap<'a> = HashMap<&'a str, HashSet<&'a str>>;

/// This normalization step flattens fragment spreads and expands inline fragments on abstract types
/// (unions and interfaces) into a series of inline fragments on concrete object types.
/// This is crucial for the query planner, which primarily operates on object types.
///
/// The process involves:
/// 1. Building a map of possible types for every union and interface in the schema.
/// 2. Traversing the query and replacing inline fragments on abstract types with inline fragments
///    for each possible concrete type.
/// 3. Handling directives on fragments by merging and propagating them downwards, ensuring
///    the correct semantics are maintained.
pub fn flatten_fragments(ctx: &mut NormalizationContext) -> Result<(), NormalizationError> {
    let possible_types = build_possible_types_map(ctx);
    let query_type_name = ctx.query_type_name();
    let mutation_type_name = ctx.mutation_type_name();
    let subscription_type_name = ctx.subscription_type_name();

    for definition in &mut ctx.document.definitions {
        if let Definition::Operation(op_def) = definition {
            let (root_type_name, selection_set) = match op_def {
                OperationDefinition::SelectionSet(s) => (query_type_name, s),
                OperationDefinition::Query(Query { selection_set, .. }) => {
                    (query_type_name, selection_set)
                }
                OperationDefinition::Mutation(Mutation { selection_set, .. }) => {
                    (mutation_type_name, selection_set)
                }
                OperationDefinition::Subscription(Subscription { selection_set, .. }) => {
                    (subscription_type_name, selection_set)
                }
            };

            let root_type_def =
                ctx.supergraph
                    .definitions
                    .get(root_type_name)
                    .ok_or_else(|| NormalizationError::SchemaTypeNotFound {
                        type_name: root_type_name.to_string(),
                    })?;

            handle_selection_set(
                ctx.supergraph,
                &possible_types,
                root_type_def,
                selection_set,
            )?;
        }
    }

    Ok(())
}

fn build_possible_types_map<'a>(ctx: &NormalizationContext<'a>) -> PossibleTypesMap<'a> {
    let mut possible_types = PossibleTypesMap::new();
    let maybe_subgraph_name = ctx.subgraph_name.as_ref();

    let relevant_definitions = ctx.supergraph.definitions.iter().filter(|(_, def)| {
        if let Some(subgraph_name) = maybe_subgraph_name {
            def.is_defined_in_subgraph(subgraph_name.as_str())
        } else {
            true
        }
    });

    for (type_name, type_def) in relevant_definitions.clone() {
        match type_def {
            SupergraphDefinition::Union(union_type) => {
                let members = union_type
                    .union_members
                    .iter()
                    .filter_map(|m| {
                        if let Some(subgraph_name) = maybe_subgraph_name {
                            if &m.graph == *subgraph_name {
                                return None;
                            }
                        }
                        Some(m.member.as_str())
                    })
                    .collect();
                possible_types.insert(type_name.as_str(), members);
            }
            SupergraphDefinition::Interface(_) => {
                let mut object_types: HashSet<&str> = HashSet::new();
                for (obj_type_name, obj_type_def) in relevant_definitions.clone() {
                    if let SupergraphDefinition::Object(object_type) = obj_type_def {
                        if object_type.join_implements.iter().any(|j| {
                            let belongs = match maybe_subgraph_name {
                                Some(subgraph_name) => &j.graph_id == *subgraph_name,
                                None => true,
                            };
                            belongs && &j.interface == type_name
                        }) {
                            object_types.insert(obj_type_name.as_str());
                        }
                    }
                }
                possible_types.insert(type_name.as_str(), object_types);
            }
            _ => {}
        }
    }
    possible_types
}

fn handle_selection_set(
    state: &SupergraphState,
    possible_types: &PossibleTypesMap,
    parent_type_def: &SupergraphDefinition,
    selection_set: &mut SelectionSet<'static, String>,
) -> Result<(), NormalizationError> {
    let old_items = std::mem::take(&mut selection_set.items);
    let mut new_items: Vec<Selection<'static, String>> = Vec::new();

    for selection in old_items {
        match selection {
            Selection::Field(mut field) => {
                process_field(state, possible_types, parent_type_def, &mut field)?;
                new_items.push(Selection::Field(field));
            }
            Selection::InlineFragment(current_fragment) => {
                let processed_fragments = process_inline_fragment(
                    state,
                    possible_types,
                    parent_type_def,
                    current_fragment,
                )?;
                new_items.extend(processed_fragments);
            }
            Selection::FragmentSpread(_) => {
                // Fragment spreads should have been inlined in a previous step.
            }
        }
    }
    selection_set.items = new_items;
    Ok(())
}

/// Processes a field's selection set recursively.
fn process_field(
    state: &SupergraphState,
    possible_types: &PossibleTypesMap,
    parent_type_def: &SupergraphDefinition,
    field: &mut Field<'static, String>,
) -> Result<(), NormalizationError> {
    if field.name.starts_with("__") || field.selection_set.items.is_empty() {
        return Ok(());
    }

    let field_def = parent_type_def.fields().get(&field.name).ok_or_else(|| {
        NormalizationError::FieldNotFoundInType {
            field_name: field.name.clone(),
            type_name: parent_type_def.name().to_string(),
        }
    })?;

    let inner_type_name = field_def.field_type.inner_type();
    let inner_type_def = state.definitions.get(inner_type_name).ok_or_else(|| {
        NormalizationError::SchemaTypeNotFound {
            type_name: inner_type_name.to_string(),
        }
    })?;

    handle_selection_set(
        state,
        possible_types,
        inner_type_def,
        &mut field.selection_set,
    )
}

fn process_inline_fragment(
    state: &SupergraphState,
    possible_types: &PossibleTypesMap,
    parent_type_def: &SupergraphDefinition,
    mut fragment: InlineFragment<'static, String>,
) -> Result<Vec<Selection<'static, String>>, NormalizationError> {
    let type_condition_matches_parent = fragment
        .type_condition
        .as_ref()
        .is_none_or(|tc| extract_type_condition(tc) == parent_type_def.name());

    if type_condition_matches_parent {
        // The fragment's type condition is the same as the parent's type, or it has no type condition.
        // We can flatten it if it has no directives, otherwise we must preserve it.
        if fragment.directives.is_empty() {
            handle_selection_set(
                state,
                possible_types,
                parent_type_def,
                &mut fragment.selection_set,
            )?;
            Ok(fragment.selection_set.items)
        } else {
            handle_selection_set(
                state,
                possible_types,
                parent_type_def,
                &mut fragment.selection_set,
            )?;
            Ok(vec![Selection::InlineFragment(fragment)])
        }
    } else {
        // The fragment has a different type condition from its parent, so we must expand it.
        expand_fragment_with_type_condition(state, possible_types, parent_type_def, fragment)
    }
}

/// Expands a fragment that has a specific type condition.
fn expand_fragment_with_type_condition(
    state: &SupergraphState,
    possible_types: &PossibleTypesMap,
    parent_type_def: &SupergraphDefinition,
    mut fragment: InlineFragment<'static, String>,
) -> Result<Vec<Selection<'static, String>>, NormalizationError> {
    let type_condition_name = fragment
        .type_condition
        .as_ref()
        .map(extract_type_condition)
        .expect("Type condition should exist here");

    let type_condition_def = state.definitions.get(type_condition_name).ok_or_else(|| {
        NormalizationError::SchemaTypeNotFound {
            type_name: type_condition_name.to_string(),
        }
    })?;

    match type_condition_def {
        SupergraphDefinition::Interface(_) | SupergraphDefinition::Union(_) => {
            expand_abstract_fragment(state, possible_types, parent_type_def, fragment)
        }
        SupergraphDefinition::Object(_) => {
            // This fragment is on a concrete object type. It's only valid if the parent
            // isn't a different, incompatible object type.
            if matches!(parent_type_def, SupergraphDefinition::Object(_))
                && parent_type_def.name() != type_condition_def.name()
            {
                // e.g. `... on Dog { ... on Cat { ... } }` -> impossible, drop inner fragment.
                return Ok(Vec::new());
            }

            handle_selection_set(
                state,
                possible_types,
                type_condition_def,
                &mut fragment.selection_set,
            )?;
            Ok(vec![Selection::InlineFragment(fragment)])
        }
        _ => {
            // Fragments cannot be defined on these types. This indicates invalid GraphQL.
            Ok(Vec::new())
        }
    }
}

/// Expands a fragment on an abstract type (interface or union) into a set of fragments
/// on concrete object types.
fn expand_abstract_fragment(
    state: &SupergraphState,
    possible_types: &PossibleTypesMap,
    parent_type_def: &SupergraphDefinition,
    fragment: InlineFragment<'static, String>,
) -> Result<Vec<Selection<'static, String>>, NormalizationError> {
    let mut new_items = Vec::new();
    let type_condition_name = extract_type_condition(
        fragment
            .type_condition
            .as_ref()
            .expect("type condition should exist"),
    );

    let object_types_of_type_cond = possible_types.get(type_condition_name).ok_or_else(|| {
        NormalizationError::PossibleTypesNotFound {
            type_name: type_condition_name.to_string(),
        }
    })?;

    let owned_parent_set;
    let object_types_of_parent_type = match parent_type_def {
        SupergraphDefinition::Union(_) | SupergraphDefinition::Interface(_) => possible_types
            .get(parent_type_def.name())
            .ok_or_else(|| NormalizationError::PossibleTypesNotFound {
                type_name: parent_type_def.name().to_string(),
            })?,
        _ => {
            owned_parent_set = HashSet::from([parent_type_def.name()]);
            &owned_parent_set
        }
    };

    let mut intersecting_types: Vec<&str> = object_types_of_type_cond
        .intersection(object_types_of_parent_type)
        .copied()
        .collect();
    intersecting_types.sort_unstable();

    for obj_type_name in intersecting_types {
        let obj_type_def = state.definitions.get(obj_type_name).ok_or_else(|| {
            NormalizationError::SchemaTypeNotFound {
                type_name: obj_type_name.to_string(),
            }
        })?;

        let inherited_fields: Vec<Selection<String>> = fragment
            .selection_set
            .items
            .iter()
            .filter(|s| matches!(s, Selection::Field(_)))
            .cloned()
            .collect();

        let specific_sub_fragment = fragment.selection_set.items.iter().find_map(|s| {
            if let Selection::InlineFragment(f) = s {
                if f.type_condition.as_ref().map(extract_type_condition) == Some(obj_type_name) {
                    return Some(f);
                }
            }
            None
        });

        if let Some(sub_fragment) = specific_sub_fragment {
            if !sub_fragment.directives.is_empty() {
                // If the sub-fragment has directives, it's treated as a distinct entity.
                // A fragment for the inherited fields (with parent directives) is created first...
                let mut inherited_fragment = InlineFragment {
                    type_condition: Some(TypeCondition::On(obj_type_name.to_string())),
                    directives: fragment.directives.clone(),
                    selection_set: SelectionSet {
                        span: fragment.selection_set.span,
                        items: inherited_fields,
                    },
                    position: fragment.position,
                };
                handle_selection_set(
                    state,
                    possible_types,
                    obj_type_def,
                    &mut inherited_fragment.selection_set,
                )?;
                new_items.push(Selection::InlineFragment(inherited_fragment));

                // then a separate fragment for the sub-fragment's fields and directives.
                let mut specific_fragment = sub_fragment.clone();
                handle_selection_set(
                    state,
                    possible_types,
                    obj_type_def,
                    &mut specific_fragment.selection_set,
                )?;
                new_items.push(Selection::InlineFragment(specific_fragment));

                continue;
            }
        }

        let mut new_fragment = InlineFragment {
            type_condition: Some(TypeCondition::On(obj_type_name.to_string())),
            directives: fragment.directives.clone(),
            selection_set: SelectionSet {
                span: fragment.selection_set.span,
                items: inherited_fields,
            },
            position: fragment.position,
        };

        if let Some(sub_fragment) = specific_sub_fragment {
            new_fragment
                .directives
                .extend(sub_fragment.directives.clone());
            new_fragment
                .selection_set
                .items
                .extend(sub_fragment.selection_set.items.clone());
        }

        handle_selection_set(
            state,
            possible_types,
            obj_type_def,
            &mut new_fragment.selection_set,
        )?;
        new_items.push(Selection::InlineFragment(new_fragment));
    }
    Ok(new_items)
}
