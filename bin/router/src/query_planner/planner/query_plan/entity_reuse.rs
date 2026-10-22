//! Finds where an entity fetch could take its data from the response instead of the subgraph.
//!
//! Say `topProducts { upc name price }` ran first, and later we need `name price` of the
//! products under `users.@.reviews.@.product`. Those are often the same products. The fetch
//! can't know which ones until it runs, but we can tell it where to look: every position an
//! earlier fetch filled with the same type and (at least) the same fields.
//! The executor matches entities there by their representation and copies the fields over.
//!
//! Fields are matched by name, arguments and response key, not by subgraph.
//! A field resolved by two subgraphs is `@shareable`, which promises the same data.

use petgraph::graph::NodeIndex;

use crate::query_planner::{
    ast::{
        requires::RequiresSelectionSet,
        selection_item::SelectionItem,
        selection_set::{InlineFragmentSelection, SelectionSet},
    },
    planner::{
        fetch::{
            fetch_graph::FetchGraph, fetch_step_data::FetchStepFlags, state::MultiTypeFetchStep,
        },
        plan_nodes::{
            planning::PlanNode, EntityReuse, FlattenNodePath, PathSegment, TypeCondition,
        },
    },
    state::supergraph_state::{OperationKind, SupergraphState, TypeNode},
};

/// Where the entity fetch at `step_index` can find entities it would otherwise fetch,
/// looking at what `earlier_steps` (steps from earlier waves) put in the response.
/// Also gives back the ids of the steps it looks at, so the fetch can wait for them.
/// None when there's nowhere to look, or the fetch is one we don't try this on.
pub(super) fn find_entity_reuse(
    fetch_graph: &FetchGraph<MultiTypeFetchStep>,
    step_index: NodeIndex,
    earlier_steps: &[NodeIndex],
    supergraph: &SupergraphState,
) -> Option<(EntityReuse, Vec<i64>)> {
    let step = fetch_graph.get_step_data(step_index).ok()?;
    if !step.is_entity_call()
        || step.operation_kind != OperationKind::Query
        || step.input_rewrites.is_some()
        || step.output_rewrites.is_some()
        || step.flags.contains(FetchStepFlags::USED_FOR_TYPE_CONDITION)
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

    let mut sources: Vec<FlattenNodePath> = Vec::new();
    let mut source_steps: Vec<i64> = Vec::new();
    for earlier_index in earlier_steps {
        let Ok(earlier) = fetch_graph.get_step_data(*earlier_index) else {
            continue;
        };
        let found_before = sources.len();
        let start = FlattenNodePath::from(earlier.response_path.clone());
        for (definition_name, selections) in earlier.output.iter_selections() {
            collect_positions(
                supergraph,
                selections,
                definition_name,
                start.as_slice().to_vec(),
                type_name,
                needed,
                &mut sources,
            );
        }
        if sources.len() > found_before {
            source_steps.push(earlier.id);
        }
    }

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

    Some((
        EntityReuse {
            sources,
            copy: RequiresSelectionSet::from(&copy),
        },
        source_steps,
    ))
}

/// Puts `reuse` on the fetch inside a node built from one fetch step.
pub(super) fn attach_entity_reuse(node: &mut PlanNode, reuse: EntityReuse) {
    match node {
        PlanNode::Flatten(flatten) => {
            if let PlanNode::Fetch(fetch) = flatten.node.as_mut() {
                fetch.entity_reuse = Some(Box::new(reuse));
            }
        }
        PlanNode::Condition(condition) => {
            if let Some(node) = condition
                .if_clause
                .as_deref_mut()
                .or(condition.else_clause.as_deref_mut())
            {
                attach_entity_reuse(node, reuse);
            }
        }
        _ => {}
    }
}

/// Walks a fetch's selections and records every position that holds `type_name` objects
/// with all the `needed` fields.
fn collect_positions(
    supergraph: &SupergraphState,
    selections: &SelectionSet,
    current_type: &str,
    path: Vec<PathSegment>,
    type_name: &str,
    needed: &SelectionSet,
    sources: &mut Vec<FlattenNodePath>,
) {
    if current_type == type_name && covers(selections, needed) {
        let path = FlattenNodePath::from(path.clone());
        if !sources.contains(&path) {
            sources.push(path);
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
                    sources,
                );
            }
            SelectionItem::InlineFragment(fragment) => {
                let mut child_path = path.clone();
                if fragment.type_condition != current_type {
                    child_path.push(PathSegment::TypeCondition(Box::new(
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
                    sources,
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
