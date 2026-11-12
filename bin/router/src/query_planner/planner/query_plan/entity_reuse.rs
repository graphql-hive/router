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

/// Tells the entity fetch in `node` where earlier fetches already put what it asks for.
pub(super) fn attach_entity_reuse(
    node: &mut PlanNode,
    step: &FetchStepData<MultiTypeFetchStep>,
    fetch_graph: &FetchGraph<MultiTypeFetchStep>,
    earlier_steps: &[NodeIndex],
    supergraph: &SupergraphState,
) {
    let PlanNode::Flatten(flatten) = node else {
        return;
    };
    let PlanNode::Fetch(fetch) = flatten.node.as_mut() else {
        return;
    };
    fetch.entity_reuse =
        find_entity_reuse(step, fetch_graph, earlier_steps, supergraph).map(Box::new);
}

fn find_entity_reuse(
    step: &FetchStepData<MultiTypeFetchStep>,
    fetch_graph: &FetchGraph<MultiTypeFetchStep>,
    earlier_steps: &[NodeIndex],
    supergraph: &SupergraphState,
) -> Option<EntityReuse> {
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
    let mut positions = Positions {
        supergraph,
        type_name,
        needed,
        representation: step.input.selections_for_definition(type_name)?,
        path: Vec::new(),
        sources: Vec::new(),
        conflicts: Vec::new(),
    };
    for earlier_index in earlier_steps {
        let Ok(earlier) = fetch_graph.get_step_data(*earlier_index) else {
            continue;
        };
        if has_context(earlier) {
            continue;
        }
        let start = FlattenNodePath::from(&earlier.response_path);
        for (definition_name, selections) in earlier.output.iter_selections() {
            positions.path.clear();
            positions.path.extend_from_slice(start.as_slice());
            positions.walk(selections, definition_name);
        }
    }
    let Positions {
        mut sources,
        conflicts,
        ..
    } = positions;
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
    })
}

/// Walks an earlier fetch's selections, collecting the positions that hold `type_name`.
struct Positions<'a> {
    supergraph: &'a SupergraphState,
    type_name: &'a str,
    needed: &'a SelectionSet,
    representation: &'a SelectionSet,
    /// where the walk is, pushed and popped on the way down and up
    path: Vec<PathSegment>,
    /// positions with every field the fetch needs
    sources: Vec<FlattenNodePath>,
    /// positions that put another field under a response key of the representation
    conflicts: Vec<FlattenNodePath>,
}

impl Positions<'_> {
    fn walk(&mut self, selections: &SelectionSet, current_type: &str) {
        if current_type == self.type_name {
            if covers(selections, self.needed) {
                push_unique(&mut self.sources, &self.path);
            }
            if conflicts_with(selections, self.representation) {
                push_unique(&mut self.conflicts, &self.path);
            }
        }

        for item in &selections.items {
            let depth = self.path.len();
            match item {
                SelectionItem::Field(field) if !field.selections.is_empty() => {
                    let Some(field_type) = self
                        .supergraph
                        .definitions
                        .get(current_type)
                        .and_then(|definition| definition.fields().get(&field.name))
                        .map(|field| &field.field_type)
                    else {
                        continue;
                    };
                    self.path.push(PathSegment::Field(
                        field.alias.as_deref().unwrap_or(&field.name).into(),
                    ));
                    for _ in 0..list_depth(field_type) {
                        self.path.push(PathSegment::List);
                    }
                    self.walk(&field.selections, field_type.inner_type());
                }
                SelectionItem::InlineFragment(fragment) => {
                    if fragment.type_condition != current_type {
                        self.path.push(PathSegment::TypeCondition(Box::new(
                            TypeCondition::from_names([fragment.type_condition.as_str()]),
                        )));
                    }
                    self.walk(&fragment.selections, &fragment.type_condition);
                }
                _ => {}
            }
            self.path.truncate(depth);
        }
    }
}

fn push_unique(list: &mut Vec<FlattenNodePath>, path: &[PathSegment]) {
    if !list.iter().any(|known| known.as_slice() == path) {
        list.push(FlattenNodePath::from(path.to_vec()));
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
