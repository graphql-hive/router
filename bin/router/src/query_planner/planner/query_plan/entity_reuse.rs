//! Finds where an entity fetch could take its data from the response instead of the subgraph:
//! every position an earlier fetch filled with the same type and (at least) the same fields.
//!
//! Fields are matched by name, arguments and response key, not by subgraph.
//! A field resolved by two subgraphs is `@shareable`, which promises the same data.
//!
//! Only in queries. Entity calls are queries even in a mutation, but there each root field can
//! change an entity an earlier one returned, so what's in the response may be stale.

use petgraph::graph::NodeIndex;

use crate::query_planner::{
    ast::{
        requires::RequiresSelectionSet,
        selection_item::SelectionItem,
        selection_set::{InlineFragmentSelection, SelectionSet},
    },
    planner::{
        fetch::{
            fetch_graph::FetchGraph,
            fetch_step_data::{FetchStepData, FetchStepFlags},
            state::MultiTypeFetchStep,
        },
        plan_nodes::{
            planning::PlanNode, EntityReuse, FlattenNodePath, PathSegment, TypeCondition,
        },
    },
    state::supergraph_state::{OperationKind, SupergraphState, TypeNode},
};

pub(super) fn find_entity_reuse(
    fetch_graph: &FetchGraph<MultiTypeFetchStep>,
    step_index: NodeIndex,
    earlier_steps: &[NodeIndex],
    supergraph: &SupergraphState,
) -> Option<EntityReuse> {
    let step = fetch_graph.get_step_data(step_index).ok()?;
    if !step.is_entity_call()
        || fetch_graph.operation_kind != OperationKind::Query
        || step.input_rewrites.is_some()
        || step.output_rewrites.is_some()
        || step.flags.contains(FetchStepFlags::USED_FOR_TYPE_CONDITION)
        || has_context(step)
    {
        return None;
    }
    let type_name = step.output.try_as_single()?;
    if step.input.try_as_single() != Some(type_name) {
        return None;
    }
    let needed = step.output.selections_for_definition(type_name)?;
    if !is_plain(needed) {
        return None;
    }
    let representation = step.input.selections_for_definition(type_name)?;

    let mut sources: Vec<FlattenNodePath> = Vec::new();
    let mut conflicts: Vec<FlattenNodePath> = Vec::new();
    for earlier_index in earlier_steps {
        let Ok(earlier) = fetch_graph.get_step_data(*earlier_index) else {
            continue;
        };
        if has_context(earlier) {
            continue;
        }
        let start = FlattenNodePath::from(earlier.response_path.clone());
        for (definition_name, selections) in earlier.output.iter_selections() {
            collect_positions(
                supergraph,
                selections,
                definition_name,
                start.as_slice().to_vec(),
                type_name,
                needed,
                representation,
                &mut sources,
                &mut conflicts,
            );
        }
    }
    // the executor matches entities by their representation, read from these positions. a user
    // alias like `id: username` would match the wrong entity there.
    sources.retain(|source| !conflicts.contains(source));

    if sources.is_empty() {
        return None;
    }

    let copy = SelectionSet {
        items: vec![SelectionItem::InlineFragment(InlineFragmentSelection {
            type_condition: type_name.to_string(),
            selections: needed.clone(),
            skip_if: None,
            include_if: None,
        })],
    };

    Some(EntityReuse {
        sources,
        copy: RequiresSelectionSet::from(&copy),
        trie: Default::default(),
    })
}

pub(super) fn attach_entity_reuse(node: &mut PlanNode, reuse: EntityReuse) {
    if let PlanNode::Flatten(flatten) = node {
        if let PlanNode::Fetch(fetch) = flatten.node.as_mut() {
            fetch.entity_reuse = Some(Box::new(reuse));
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn collect_positions(
    supergraph: &SupergraphState,
    selections: &SelectionSet,
    current_type: &str,
    path: Vec<PathSegment>,
    type_name: &str,
    needed: &SelectionSet,
    representation: &SelectionSet,
    sources: &mut Vec<FlattenNodePath>,
    conflicts: &mut Vec<FlattenNodePath>,
) {
    if current_type == type_name {
        let positions = [
            (covers(selections, needed), &mut *sources),
            (conflicts_with(selections, representation), &mut *conflicts),
        ];
        for (found, list) in positions {
            let path = FlattenNodePath::from(path.clone());
            if found && !list.contains(&path) {
                list.push(path);
            }
        }
    }

    for item in &selections.items {
        match item {
            SelectionItem::Field(field) if !field.selections.is_empty() => {
                let Some(field_type) = supergraph
                    .definitions
                    .get(current_type)
                    .and_then(|definition| definition.fields().get(&field.name))
                    .map(|field| &field.field_type)
                else {
                    continue;
                };
                let mut child_path = path.clone();
                child_path.push(PathSegment::Field(
                    field.alias.as_deref().unwrap_or(&field.name).into(),
                ));
                for _ in 0..list_depth(field_type) {
                    child_path.push(PathSegment::List);
                }
                collect_positions(
                    supergraph,
                    &field.selections,
                    field_type.inner_type(),
                    child_path,
                    type_name,
                    needed,
                    representation,
                    sources,
                    conflicts,
                );
            }
            SelectionItem::InlineFragment(fragment) => {
                let mut child_path = path.clone();
                if fragment.type_condition != current_type {
                    child_path.push(PathSegment::TypeCondition(std::sync::Arc::new(
                        TypeCondition::from_names([fragment.type_condition.as_str()]),
                    )));
                }
                collect_positions(
                    supergraph,
                    &fragment.selections,
                    &fragment.type_condition,
                    child_path,
                    type_name,
                    needed,
                    representation,
                    sources,
                    conflicts,
                );
            }
            _ => {}
        }
    }
}

/// Does `source` ask for every field in `needed`, with the same response key and arguments?
/// `__typename` always counts: the executor checks it's really there before copying.
fn covers(source: &SelectionSet, needed: &SelectionSet) -> bool {
    needed.items.iter().all(|item| {
        let SelectionItem::Field(needed_field) = item else {
            return false;
        };
        if needed_field.name == "__typename" {
            return true;
        }
        let key = needed_field.alias.as_ref().unwrap_or(&needed_field.name);
        source.items.iter().any(|item| {
            let SelectionItem::Field(field) = item else {
                return false;
            };
            field.alias.as_ref().unwrap_or(&field.name) == key
                && field.name == needed_field.name
                && field.arguments == needed_field.arguments
                && covers(&field.selections, &needed_field.selections)
        })
    })
}

/// Does `selections` put a different field (or arguments) under a response key `representation`
/// reads?
fn conflicts_with(selections: &SelectionSet, representation: &SelectionSet) -> bool {
    representation.items.iter().any(|item| {
        let SelectionItem::Field(wanted) = item else {
            return false;
        };
        let key = wanted.alias.as_ref().unwrap_or(&wanted.name);
        selections.items.iter().any(|item| {
            let SelectionItem::Field(field) = item else {
                return false;
            };
            field.alias.as_ref().unwrap_or(&field.name) == key
                && (field.name != wanted.name
                    || field.arguments != wanted.arguments
                    || conflicts_with(&field.selections, &wanted.selections))
        })
    })
}

/// Fields of a fetch with `@fromContext` arguments depend on values from outside the entity,
/// so the same entity can come back different. We don't copy into or out of those.
fn has_context(step: &FetchStepData<MultiTypeFetchStep>) -> bool {
    !step.context_variable_definitions.is_empty() || step.context_rewrites.is_some()
}

/// Only plain fields, all the way down. Fragments and `@skip`/`@include` would need the
/// copy to know about types and variables, which we don't do (yet).
fn is_plain(selections: &SelectionSet) -> bool {
    selections.items.iter().all(|item| match item {
        SelectionItem::Field(field) => {
            field.skip_if.is_none() && field.include_if.is_none() && is_plain(&field.selections)
        }
        _ => false,
    })
}

fn list_depth(type_node: &TypeNode) -> usize {
    match type_node {
        TypeNode::List(inner) => 1 + list_depth(inner),
        TypeNode::NonNull(inner) => list_depth(inner),
        TypeNode::Named(_) => 0,
    }
}
