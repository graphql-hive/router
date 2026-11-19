//! Takes the fields of a nested entity, past its key, out of a fetch and into a follow-up entity
//! fetch to the same subgraph. Only where another fetch puts those fields of that type in the
//! response, so entity reuse copies them and the subgraph is never asked.
//!
//! A subgraph asked for `users { reviews { product { reviews { author { reviews { body } } } } } }`
//! in one go resolves and sends the same reviews over and over. Split, it sends each user's and
//! product's reviews once, and the router copies them into the deeper spots. If an entity isn't
//! in the response after all, the follow-up asks the subgraph for it, deduplicated.
//!
//! The fetch graph stays as it is. The plan sends the trimmed host, and the executor runs the
//! follow-ups right after it, as part of it: whatever waits for the host waits for them too.
//!
//! Only runs with entity reuse on, and only in queries.

use std::{collections::BTreeSet, rc::Rc};

use ahash::{HashMap, HashMapExt, HashSet, HashSetExt};
use petgraph::{algo::toposort, graph::NodeIndex, visit::EdgeRef};

use crate::query_planner::{
    ast::{
        merge_path::{FieldPathSegment, MergePath, Segment},
        selection_item::SelectionItem,
        selection_set::{FieldSelection, SelectionSet},
    },
    planner::{
        fetch::{
            error::FetchGraphError,
            fetch_graph::FetchGraph,
            fetch_step_data::{FetchStepData, FetchStepFlags, FetchStepKind},
            selections::FetchStepSelections,
            state::MultiTypeFetchStep,
        },
        query_plan::entity_reuse::{
            covers, has_context, is_plain, resolved_by, resolves_every_field,
        },
    },
    state::supergraph_state::{OperationKind, SupergraphDefinition, SupergraphState, TypeNode},
};

type Step = FetchStepData<MultiTypeFetchStep>;

/// A field to split, by where it is in its fetch's selection
#[derive(Clone, PartialEq, Eq, Hash)]
struct SplitKey {
    host: NodeIndex,
    /// item indexes, from the fetch's selection down to the field
    items: Vec<usize>,
}

struct Split {
    key: SplitKey,
    response_path: MergePath,
    type_name: String,
    /// what the new fetch sends: `__typename` and the key
    representation: SelectionSet,
    /// what the new fetch asks for, what the host no longer does
    moved: SelectionSet,
}

/// A host fetch with nested entities taken out, and the follow-up fetches that fill them in
pub(crate) struct FollowUps {
    pub host: NodeIndex,
    /// what the host sends instead, its key fields stay
    pub trimmed: Step,
    /// by level, each fills in under the keys the one before put there. The host counts as done
    /// once the last level is.
    pub levels: Vec<Vec<Step>>,
}

pub(crate) fn plan_follow_ups(
    graph: &mut FetchGraph<MultiTypeFetchStep>,
    supergraph: &SupergraphState,
) -> Result<Vec<FollowUps>, FetchGraphError> {
    if graph.operation_kind != OperationKind::Query {
        return Ok(Vec::new());
    }
    let Some(root) = graph.root_index else {
        return Ok(Vec::new());
    };
    let depth = depths(graph, root);
    let hosts: Vec<NodeIndex> = graph
        .step_indices()
        .filter(|&index| index != root)
        .filter(|&index| graph.get_step_data(index).is_ok_and(can_host))
        .collect();
    if hosts.is_empty() {
        return Ok(Vec::new());
    }

    // Start by splitting everything that could be. A split without a source would only cost a
    // call, so it goes back in, which changes what the others can copy from. Splits only ever go
    // back in, so this ends.
    let mut kept_in: HashSet<SplitKey> = HashSet::new();
    let (mut trimmed, splits) = loop {
        let mut trimmed: HashMap<NodeIndex, SelectionSet> = HashMap::new();
        let mut splits = Vec::new();
        for &host in &hosts {
            let step = graph.get_step_data(host)?;
            let (type_name, selections) = single(step).expect("hosts have one type");
            let mut items = Vec::new();
            let selections = trim(
                supergraph,
                step,
                host,
                type_name,
                selections,
                &step.response_path,
                &mut items,
                &kept_in,
                &mut splits,
            );
            trimmed.insert(host, selections);
        }

        let mut changed = false;
        for split in &splits {
            if !has_source(graph, supergraph, &depth, &trimmed, split) {
                kept_in.insert(split.key.clone());
                changed = true;
            }
        }
        if !changed {
            break (trimmed, splits);
        }
    };

    let mut follow_ups = Vec::new();
    // in graph order, so follow-ups get the same ids every time
    for host in hosts {
        let Some(selections) = trimmed.remove(&host) else {
            continue;
        };
        let mine: Vec<&Split> = splits.iter().filter(|s| s.key.host == host).collect();
        if mine.is_empty() {
            continue;
        }
        let step = graph.get_step_data(host)?.clone();
        let mut trimmed = step.clone();
        let type_name = step.output.try_as_single().expect("hosts have one type");
        *trimmed
            .output
            .selections_for_definition_mut(type_name)
            .expect("hosts have one type") = selections;
        fit_variables(&mut trimmed, &step);

        let mut levels: Vec<Vec<Step>> = Vec::new();
        for split in &mine {
            // how many other splits it's inside of
            let level = mine
                .iter()
                .filter(|other| in_region(&split.response_path, other))
                .count();
            if levels.len() <= level {
                levels.resize_with(level + 1, Vec::new);
            }
            let mut input = FetchStepSelections::new(&split.type_name);
            input
                .add(&split.representation)
                .map_err(|e| FetchGraphError::Internal(format!("split representation: {e}")))?;
            let mut output = FetchStepSelections::new(&split.type_name);
            output
                .add(&split.moved)
                .map_err(|e| FetchGraphError::Internal(format!("split selection: {e}")))?;
            let mut follow_up = FetchStepData {
                id: graph.create_fetch_id(),
                service_name: step.service_name.clone(),
                response_path: split.response_path.clone(),
                input: input.into_multi_type(),
                output: output.into_multi_type(),
                kind: FetchStepKind::Entity,
                operation_kind: OperationKind::Query,
                flags: FetchStepFlags::empty(),
                condition: None,
                gated_by: Vec::new(),
                variable_usages: None,
                variable_definitions: None,
                mutation_field_position: None,
                input_rewrites: None,
                output_rewrites: None,
                context_rewrites: None,
                context_variable_definitions: Vec::new(),
                context_producers: BTreeSet::new(),
            };
            fit_variables(&mut follow_up, &step);
            levels[level].push(follow_up);
        }
        follow_ups.push(FollowUps {
            host,
            trimmed,
            levels,
        });
    }
    Ok(follow_ups)
}

/// The variables `step` uses, defined like the host defines them
fn fit_variables(step: &mut Step, host: &Step) {
    let usages = step.output.variable_usages();
    step.variable_definitions = host.variable_definitions.as_ref().map(|definitions| {
        definitions
            .iter()
            .filter(|definition| usages.contains(&definition.name))
            .cloned()
            .collect()
    });
    step.variable_usages = (!usages.is_empty()).then_some(usages);
}

/// A fetch whose nested entities may be split out
fn can_host(step: &Step) -> bool {
    step.operation_kind == OperationKind::Query
        && step.flags.is_empty()
        && step.condition.is_none()
        && step.gated_by.is_empty()
        && step.input_rewrites.is_none()
        && step.output_rewrites.is_none()
        && !has_context(step)
        && single(step).is_some()
}

fn single(step: &Step) -> Option<(&str, &SelectionSet)> {
    let type_name = step.output.try_as_single()?;
    Some((type_name, step.output.selections_for_definition(type_name)?))
}

/// The selection of the host once every nested entity that can be is split out. The splits go to
/// `splits`, inner ones first.
#[allow(clippy::too_many_arguments)]
fn trim(
    supergraph: &SupergraphState,
    step: &Step,
    host: NodeIndex,
    type_name: &str,
    selections: &SelectionSet,
    response_path: &MergePath,
    items: &mut Vec<usize>,
    kept_in: &HashSet<SplitKey>,
    splits: &mut Vec<Split>,
) -> SelectionSet {
    let mut out = selections.clone();
    for (index, item) in selections.items.iter().enumerate() {
        let SelectionItem::Field(field) = item else {
            continue;
        };
        if field.selections.is_empty() {
            continue;
        }
        let Some(definition) = supergraph
            .definitions
            .get(type_name)
            .and_then(|definition| definition.fields().get(&field.name))
        else {
            continue;
        };
        let inner = definition.field_type.inner_type();
        let mut path = response_path.push(Segment::Field(
            Rc::new(FieldPathSegment::new(
                field.name.clone(),
                field.alias.clone(),
            )),
            field.arguments_hash(),
            None,
        ));
        for _ in 0..list_depth(&definition.field_type) {
            path = path.push(Segment::List);
        }
        items.push(index);
        let children = trim(
            supergraph,
            step,
            host,
            inner,
            &field.selections,
            &path,
            items,
            kept_in,
            splits,
        );
        let key = SplitKey {
            host,
            items: items.clone(),
        };
        items.pop();

        let SelectionItem::Field(out_field) = &mut out.items[index] else {
            unreachable!("same items");
        };
        if field.skip_if.is_some() || field.include_if.is_some() || kept_in.contains(&key) {
            out_field.selections = children;
            continue;
        }
        match split_at(supergraph, &step.service_name.0, inner, &children) {
            Some((kept, representation, moved)) => {
                out_field.selections = kept;
                splits.push(Split {
                    key,
                    response_path: path,
                    type_name: inner.to_string(),
                    representation,
                    moved,
                });
            }
            None => out_field.selections = children,
        }
    }
    out
}

/// What stays in the host, what the split fetch sends, and what it asks for. `None` when the
/// type isn't an entity of `subgraph` or there's nothing past the key to move.
fn split_at(
    supergraph: &SupergraphState,
    subgraph: &str,
    type_name: &str,
    selections: &SelectionSet,
) -> Option<(SelectionSet, SelectionSet, SelectionSet)> {
    if !matches!(
        supergraph.definitions.get(type_name),
        Some(SupergraphDefinition::Object(_))
    ) {
        return None;
    }
    let subgraph_index = supergraph.subgraph_index(subgraph)?;
    let fields: Vec<&FieldSelection> = selections
        .items
        .iter()
        .map(|item| match item {
            SelectionItem::Field(field) => Some(field),
            _ => None,
        })
        .collect::<Option<_>>()?;
    let plain = |field: &FieldSelection| field.alias.is_none() && field.selections.is_empty();
    let present = |name: &String| {
        fields
            .iter()
            .any(|field| plain(field) && &field.name == name && field.arguments.is_none())
    };
    // a key whose every field is there as a plain leaf. Adding a missing one isn't safe: its
    // response key may hold another field, from an alias another fetch fills (`id: name`)
    let key = supergraph
        .entity_keys
        .of_subgraph(subgraph)?
        .get(type_name)?
        .iter()
        .find(|key| key.iter().all(&present))?;
    let is_key = |field: &FieldSelection| field.alias.is_none() && key.contains(&field.name);

    let mut kept = SelectionSet::default();
    let mut representation = SelectionSet::default();
    let mut moved = SelectionSet::default();
    kept.items
        .push(SelectionItem::Field(FieldSelection::new_typename()));
    representation
        .items
        .push(SelectionItem::Field(FieldSelection::new_typename()));
    for field in fields {
        if field.name == "__typename" {
            if field.alias.is_some() {
                return None;
            }
            continue;
        }
        if is_key(field) {
            kept.items.push(SelectionItem::Field(field.clone()));
            representation
                .items
                .push(SelectionItem::Field(field.clone()));
        } else if resolved_by(supergraph, type_name, &field.name, subgraph_index).is_some() {
            moved.items.push(SelectionItem::Field(field.clone()));
        } else {
            // `@provides`: the subgraph only has it here, not when asked for the entity
            kept.items.push(SelectionItem::Field(field.clone()));
        }
    }
    if moved.items.is_empty() || !is_plain(&moved) {
        return None;
    }
    Some((kept, representation, moved))
}

/// Whether a fetch puts every moved field in the response by the time the host is done, as far
/// as the plan tells: the host's own other spots, or a fetch planned no later that doesn't wait
/// for it. The executor copies whatever is really there and asks for the rest.
fn has_source(
    graph: &FetchGraph<MultiTypeFetchStep>,
    supergraph: &SupergraphState,
    depth: &HashMap<NodeIndex, usize>,
    trimmed: &HashMap<NodeIndex, SelectionSet>,
    split: &Split,
) -> bool {
    let host = split.key.host;
    let host_depth = depth.get(&host).copied().unwrap_or(0);
    let Ok(host_step) = graph.get_step_data(host) else {
        return false;
    };
    graph.step_indices().any(|index| {
        if Some(index) == graph.root_index
            || (index != host
                && (graph.is_descendant_of(index, host)
                    || depth.get(&index).copied().unwrap_or(usize::MAX) > host_depth))
        {
            return false;
        }
        let Ok(step) = graph.get_step_data(index) else {
            return false;
        };
        if step.operation_kind != OperationKind::Query || has_context(step) {
            return false;
        }
        // an entity fetch that reads the key from somewhere else, `id: _internal_qp_alias_0`
        if step
            .input
            .selections_for_definition(&split.type_name)
            .is_some_and(|input| renames_key(input, &split.representation))
        {
            return false;
        }
        let Some(subgraph) = supergraph.subgraph_index(&step.service_name.0) else {
            return false;
        };
        // the same subgraph sends what it put there, `@provides` included, see `find_in`
        if step.service_name != host_step.service_name
            && !resolves_every_field(supergraph, &split.type_name, &split.moved, subgraph)
        {
            return false;
        }
        let holds = |type_name: &str, selections: &SelectionSet| {
            holds_position(
                supergraph,
                type_name,
                selections,
                &split.type_name,
                &split.representation,
                &split.moved,
            )
        };
        match trimmed.get(&index) {
            Some(selections) => holds(step.output.try_as_single().unwrap_or_default(), selections),
            None => step
                .output
                .iter_selections()
                .any(|(type_name, selections)| holds(type_name, selections)),
        }
    })
}

/// Whether any object of `wanted_type` in `selections` (of `type_name`) has every `needed` field,
/// through plain fields only, and doesn't rename its key. A key under an alias, like
/// `id: username`, never matches a follow-up's representation, so it'd always be a call.
fn holds_position(
    supergraph: &SupergraphState,
    type_name: &str,
    selections: &SelectionSet,
    wanted_type: &str,
    representation: &SelectionSet,
    needed: &SelectionSet,
) -> bool {
    if type_name == wanted_type
        && covers(selections, needed)
        && !renames_key(selections, representation)
    {
        return true;
    }
    selections.items.iter().any(|item| match item {
        SelectionItem::Field(field) if !field.selections.is_empty() => supergraph
            .definitions
            .get(type_name)
            .and_then(|definition| definition.fields().get(&field.name))
            .is_some_and(|definition| {
                holds_position(
                    supergraph,
                    definition.field_type.inner_type(),
                    &field.selections,
                    wanted_type,
                    representation,
                    needed,
                )
            }),
        _ => false,
    })
}

/// Whether a field of `selections` has a key field of `representation` under another response
/// key, or another field under a key field's: `x: id` or `id: x`
fn renames_key(selections: &SelectionSet, representation: &SelectionSet) -> bool {
    let is_key = |name: &str| {
        representation.items.iter().any(
            |item| matches!(item, SelectionItem::Field(key) if key.name != "__typename" && key.name == name),
        )
    };
    selections.items.iter().any(|item| {
        let SelectionItem::Field(field) = item else {
            return false;
        };
        let response_key = field.selection_identifier();
        field.name != response_key && (is_key(&field.name) || is_key(response_key))
    })
}

/// Whether `path` is inside what `split` moved: below its spot, under one of the moved fields
fn in_region(path: &MergePath, split: &Split) -> bool {
    let prefix = split.response_path.inner.as_ref();
    let path = path.inner.as_ref();
    if path.len() <= prefix.len() || !same_segments(&path[..prefix.len()], prefix) {
        return false;
    }
    let Segment::Field(field, _, _) = &path[prefix.len()] else {
        return false;
    };
    split.moved.items.iter().any(|item| {
        matches!(item, SelectionItem::Field(moved) if moved.selection_identifier() == field.response_key())
    })
}

fn same_segments(a: &[Segment], b: &[Segment]) -> bool {
    a.iter().zip(b).all(|pair| match pair {
        (Segment::Field(a, _, _), Segment::Field(b, _, _)) => a.response_key() == b.response_key(),
        (Segment::List, Segment::List) => true,
        (Segment::TypeCondition(a, _), Segment::TypeCondition(b, _)) => a == b,
        _ => false,
    })
}

/// How many fetches each step waits for in a row, its wave in the plan
fn depths(graph: &FetchGraph<MultiTypeFetchStep>, root: NodeIndex) -> HashMap<NodeIndex, usize> {
    let mut depth = HashMap::new();
    depth.insert(root, 0);
    // the graph has no cycles here, a cycle fails planning later anyway
    for index in toposort(&graph.graph, None).unwrap_or_default() {
        let own = graph
            .parents_of(index)
            .filter_map(|edge| depth.get(&edge.source()).map(|d| d + 1))
            .max()
            .unwrap_or(0);
        depth.insert(index, own);
    }
    depth
}

fn list_depth(type_node: &TypeNode) -> usize {
    match type_node {
        TypeNode::List(inner) => 1 + list_depth(inner),
        TypeNode::NonNull(inner) => list_depth(inner),
        TypeNode::Named(_) => 0,
    }
}
