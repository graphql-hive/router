use std::collections::{BTreeMap, BTreeSet};

use tracing::instrument;

use crate::{
    ast::{
        selection_item::SelectionItem,
        selection_set::{InlineFragmentSelection, SelectionSet},
    },
    planner::fetch::{
        error::FetchGraphError, fetch_graph::FetchGraph, selections::FetchStepSelections,
        state::MultiTypeFetchStep,
    },
    state::supergraph_state::{SupergraphDefinition, SupergraphState},
};

impl FetchGraph<MultiTypeFetchStep> {
    /// Per-subgraph supertype collapse pass.
    ///
    /// For each fetch step, rewrites concrete-keyed fan-outs back to an abstract supertype
    /// when the target subgraph defines that abstract type and the concrete set exactly
    /// covers its runtime objects there. Applies to both selection map keys and inline
    /// fragments at every nesting level. Returns `true` when anything was rewritten so the
    /// optimize loop can re-run sibling passes that may now find new merge candidates.
    #[instrument(level = "trace", skip_all)]
    pub(crate) fn collapse_supertype_per_subgraph(
        &mut self,
        supergraph_state: &SupergraphState,
    ) -> Result<bool, FetchGraphError> {
        let mut mutated = false;

        let step_indices: Vec<_> = self.step_indices().collect();

        for index in step_indices {
            let step = self.get_step_data(index)?;
            let subgraph_name = step.service_name.0.clone();

            let Some(graph_id) =
                supergraph_state
                    .known_subgraphs
                    .iter()
                    .find_map(|(graph_id, name)| {
                        if name == &subgraph_name {
                            Some(graph_id.clone())
                        } else {
                            None
                        }
                    })
            else {
                // Synthetic / non-subgraph steps have no schema to validate against.
                continue;
            };

            let step = self.get_step_data_mut(index)?;
            if collapse_step_selections(&mut step.input, supergraph_state, &graph_id) {
                mutated = true;
            }
            if collapse_step_selections(&mut step.output, supergraph_state, &graph_id) {
                mutated = true;
            }
        }

        Ok(mutated)
    }
}

/// Runs both rewrites of the per-subgraph supertype collapse on a single fetch step's
/// selection map and returns `true` when anything changed.
fn collapse_step_selections(
    step_selections: &mut FetchStepSelections<MultiTypeFetchStep>,
    supergraph: &SupergraphState,
    graph_id: &str,
) -> bool {
    let mut mutated = try_collapse_keys_to_supertype(step_selections, supergraph, graph_id);

    for (parent_type_name, selection_set) in step_selections.iter_mut() {
        if walk_and_collapse(selection_set, supergraph, graph_id, Some(parent_type_name)) {
            mutated = true;
        }
    }

    mutated
}

/// Top-level map-key collapse: rewrites `{ ConcreteA: S, ConcreteB: S, ConcreteC: S }` to
/// `{ Abstract: S }` when the concrete set exactly covers an abstract supertype's runtime
/// objects in `graph_id`. Root operation types are skipped (they are positional, not reachable
/// through an abstract).
fn try_collapse_keys_to_supertype(
    step_selections: &mut FetchStepSelections<MultiTypeFetchStep>,
    supergraph: &SupergraphState,
    graph_id: &str,
) -> bool {
    let entries: Vec<(&String, &SelectionSet)> = step_selections.iter().collect();

    if entries.len() < 2 {
        return false;
    }
    if entries
        .iter()
        .any(|(k, _)| matches!(k.as_str(), "Query" | "Mutation" | "Subscription"))
    {
        return false;
    }

    let concrete_set: BTreeSet<&str> = entries.iter().map(|(k, _)| k.as_str()).collect();
    for key in concrete_set.iter() {
        if !matches!(
            supergraph.definitions.get(*key),
            Some(SupergraphDefinition::Object(_))
        ) {
            return false;
        }
    }

    let mut iter = entries.iter().map(|(_, v)| *v);
    let shared = iter.next().expect("len >= 2 checked above");
    for other in iter {
        if other != shared {
            return false;
        }
    }
    let shared_clone = shared.clone();

    let Some(abstract_name) =
        find_matching_abstract_type(supergraph, graph_id, &concrete_set, &shared_clone)
    else {
        return false;
    };

    let mut new_map = BTreeMap::<String, SelectionSet>::new();
    new_map.insert(abstract_name.to_string(), shared_clone);
    step_selections.replace_with(new_map);
    true
}

/// Recursive walk: at each level strips redundant `... on Same` fragments (where `Same` is
/// the static parent type) and folds matching concrete-fragment groups into a single
/// `... on Abstract` fragment. `parent_type_name` is the static parent type; when `None` the
/// parent-driven steps are skipped. Returns `true` if anything was rewritten at this level or
/// below.
fn walk_and_collapse(
    selection_set: &mut SelectionSet,
    supergraph: &SupergraphState,
    graph_id: &str,
    parent_type_name: Option<&str>,
) -> bool {
    let mut mutated = false;

    if let Some(parent) = parent_type_name {
        if flatten_self_fragments_at_level(&mut selection_set.items, parent) {
            mutated = true;
        }
    }

    let parent_type_def = parent_type_name.and_then(|name| supergraph.definitions.get(name));

    for item in selection_set.items.iter_mut() {
        match item {
            SelectionItem::Field(field) => {
                if field.name.starts_with("__") {
                    continue;
                }
                let Some(parent_def) = parent_type_def else {
                    continue;
                };
                let Some(field_def) = parent_def.fields().get(&field.name) else {
                    continue;
                };
                let inner_type_name = field_def.field_type.inner_type();
                if walk_and_collapse(
                    &mut field.selections,
                    supergraph,
                    graph_id,
                    Some(inner_type_name),
                ) {
                    mutated = true;
                }
            }
            SelectionItem::InlineFragment(fragment) => {
                if walk_and_collapse(
                    &mut fragment.selections,
                    supergraph,
                    graph_id,
                    Some(fragment.type_condition.as_str()),
                ) {
                    mutated = true;
                }
            }
            SelectionItem::FragmentSpread(_) => {}
        }
    }

    if let Some(collapsed) = try_collapse_concrete_fragments_at_level(
        &selection_set.items,
        supergraph,
        graph_id,
        parent_type_name,
    ) {
        selection_set.items = collapsed;
        mutated = true;
    }

    mutated
}

/// Inlines `... on ParentType` fragments at this level (deduping against existing items) and
/// returns `true` if any were inlined. Fragments with `@skip` / `@include` are kept as-is.
fn flatten_self_fragments_at_level(items: &mut Vec<SelectionItem>, parent_type: &str) -> bool {
    if !items.iter().any(|item| match item {
        SelectionItem::InlineFragment(fragment) => {
            fragment.type_condition == parent_type
                && fragment.include_if.is_none()
                && fragment.skip_if.is_none()
        }
        _ => false,
    }) {
        return false;
    }

    let original = std::mem::take(items);
    let mut new_items: Vec<SelectionItem> = Vec::with_capacity(original.len());

    for item in original {
        match item {
            SelectionItem::InlineFragment(fragment)
                if fragment.type_condition == parent_type
                    && fragment.include_if.is_none()
                    && fragment.skip_if.is_none() =>
            {
                for inner in fragment.selections.items {
                    if !new_items.contains(&inner) {
                        new_items.push(inner);
                    }
                }
            }
            other => {
                if !new_items.contains(&other) {
                    new_items.push(other);
                }
            }
        }
    }

    *items = new_items;
    true
}

fn try_collapse_concrete_fragments_at_level(
    items: &[SelectionItem],
    supergraph: &SupergraphState,
    graph_id: &str,
    parent_type_name: Option<&str>,
) -> Option<Vec<SelectionItem>> {
    let mut collapsible_indices: Vec<usize> = Vec::new();
    let mut concrete_to_selection: BTreeMap<&str, &SelectionSet> = BTreeMap::new();

    for (idx, item) in items.iter().enumerate() {
        let SelectionItem::InlineFragment(fragment) = item else {
            continue;
        };
        if fragment.include_if.is_some() || fragment.skip_if.is_some() {
            continue;
        }
        let Some(SupergraphDefinition::Object(_)) =
            supergraph.definitions.get(&fragment.type_condition)
        else {
            continue;
        };
        if concrete_to_selection
            .insert(fragment.type_condition.as_str(), &fragment.selections)
            .is_some()
        {
            // Duplicate concrete keys at the same level: skip rather than try to merge them.
            return None;
        }
        collapsible_indices.push(idx);
    }

    if collapsible_indices.len() < 2 {
        return None;
    }

    let mut iter = concrete_to_selection.values();
    let shared = iter.next()?;
    for other in iter {
        if **other != **shared {
            return None;
        }
    }
    let shared_selection_set = (*shared).clone();

    let concrete_set: BTreeSet<&str> = concrete_to_selection.keys().copied().collect();

    let abstract_name = find_matching_abstract_type(supergraph, graph_id, &concrete_set, shared)?;

    let collapsible_set: BTreeSet<usize> = collapsible_indices.iter().copied().collect();

    let first_idx = *collapsible_indices.first()?;
    let mut new_items: Vec<SelectionItem> = Vec::with_capacity(items.len());
    let mut placed_collapsed = false;

    let abstract_matches_parent = parent_type_name == Some(abstract_name);

    for (idx, item) in items.iter().enumerate() {
        if collapsible_set.contains(&idx) {
            if idx == first_idx && !placed_collapsed {
                placed_collapsed = true;
                if abstract_matches_parent {
                    for inner in shared_selection_set.items.iter() {
                        if !new_items.contains(inner)
                            && !is_already_present_later(items, idx, inner, &collapsible_set)
                        {
                            new_items.push(inner.clone());
                        }
                    }
                } else {
                    new_items.push(SelectionItem::InlineFragment(InlineFragmentSelection {
                        type_condition: abstract_name.to_string(),
                        include_if: None,
                        skip_if: None,
                        selections: shared_selection_set.clone(),
                    }));
                }
            }
            continue;
        }
        new_items.push(item.clone());
    }

    Some(new_items)
}

fn is_already_present_later(
    items: &[SelectionItem],
    from_idx: usize,
    candidate: &SelectionItem,
    collapsible_set: &BTreeSet<usize>,
) -> bool {
    items
        .iter()
        .enumerate()
        .skip(from_idx + 1)
        .any(|(idx, item)| !collapsible_set.contains(&idx) && item == candidate)
}

/// Finds an abstract supertype in `graph_id` whose runtime objects exactly match
/// `concrete_set` and that defines every selected field in `shared_selections`. When several
/// supertypes match, returns the lexicographically first name for plan stability.
fn find_matching_abstract_type<'a>(
    supergraph: &'a SupergraphState,
    graph_id: &str,
    concrete_set: &BTreeSet<&str>,
    shared_selections: &SelectionSet,
) -> Option<&'a str> {
    let mut candidate_names: Vec<&str> =
        supergraph.definitions.keys().map(|s| s.as_str()).collect();
    candidate_names.sort_unstable();

    for type_name in candidate_names {
        let Some(type_def) = supergraph.definitions.get(type_name) else {
            continue;
        };
        if !type_def.is_defined_in_subgraph(graph_id) {
            continue;
        }
        let possible_types: BTreeSet<&str> = match type_def {
            SupergraphDefinition::Interface(_) => supergraph
                .definitions
                .iter()
                .filter_map(|(obj_name, obj_def)| {
                    let SupergraphDefinition::Object(obj) = obj_def else {
                        return None;
                    };
                    let implements_in_subgraph = obj
                        .join_implements
                        .iter()
                        .any(|j| j.graph_id == graph_id && j.interface == type_name);
                    if implements_in_subgraph {
                        Some(obj_name.as_str())
                    } else {
                        None
                    }
                })
                .collect(),
            SupergraphDefinition::Union(union_type) => union_type
                .union_members
                .iter()
                .filter_map(|m| {
                    if m.graph == graph_id {
                        Some(m.member.as_str())
                    } else {
                        None
                    }
                })
                .collect(),
            _ => continue,
        };

        if possible_types.is_empty() || &possible_types != concrete_set {
            continue;
        }

        if !shared_selections_compatible_with(type_def, graph_id, shared_selections) {
            continue;
        }

        return Some(type_name);
    }
    None
}

/// Whether every selected non-`__typename` field in `shared_selections` is defined on
/// `abstract_type` *as seen by `graph_id`* (filtered through `@join__field`). A
/// supergraph-wide check would accept fields that the target subgraph only carries on
/// concrete implementors, producing a `... on Abstract { field }` that the subgraph rejects.
/// Unions are always rejected here since they don't declare fields.
fn shared_selections_compatible_with(
    abstract_type: &SupergraphDefinition,
    graph_id: &str,
    shared_selections: &SelectionSet,
) -> bool {
    for item in shared_selections.items.iter() {
        match item {
            SelectionItem::Field(field) => {
                if field.name == "__typename" {
                    continue;
                }
                let defined_on_abstract_in_subgraph = match abstract_type {
                    SupergraphDefinition::Object(o) => {
                        o.fields_of_subgraph(graph_id).contains_key(&field.name)
                    }
                    SupergraphDefinition::Interface(i) => {
                        i.fields_of_subgraph(graph_id).contains_key(&field.name)
                    }
                    SupergraphDefinition::Union(_) => false,
                    _ => false,
                };
                if !defined_on_abstract_in_subgraph {
                    return false;
                }
            }
            SelectionItem::InlineFragment(_) | SelectionItem::FragmentSpread(_) => {
                // Carries its own type condition; not validated against the parent abstract.
            }
        }
    }

    true
}
