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
    /// For each fetch step, looks at its target subgraph and tries two complementary rewrites
    /// on its `input` and `output` selection maps:
    ///
    /// - When the map keys are concrete object types whose set exactly matches the runtime
    ///   objects of an abstract supertype defined in that subgraph, and every per-key
    ///   selection set is identical, the map collapses to a single entry keyed by that
    ///   abstract type. Subsequent passes (`merge_siblings`, `batch_multi_type`) then see the
    ///   abstract-keyed shape and may unlock further plan simplifications.
    /// - At every level inside each kept selection set, groups of inline fragments on
    ///   different concrete object types that carry the same selections and together cover an
    ///   abstract supertype's runtime objects in that subgraph fold into a single
    ///   `... on Abstract { ... }` fragment, with equal `... on Same { ... }` wrappers
    ///   removed at the surrounding parent level.
    ///
    /// The pass is gated on the same per-subgraph predicates the rendered subgraph fetch
    /// document needs to remain valid: the abstract type must be defined in the target
    /// subgraph, every selected non-`__typename` field on the abstract fragment must exist on
    /// that abstract type, and the concrete set must cover exactly the abstract type's
    /// implementors / union members in that subgraph (no more, no less).
    ///
    /// Returns `true` when at least one selection map was rewritten, so the optimize loop
    /// knows to schedule another iteration of the surrounding passes (`merge_siblings`,
    /// `batch_multi_type`, etc.) that may now find new merge candidates because of the
    /// abstract-keyed shape.
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
                // Synthetic root steps (or any step whose service name is not a real subgraph)
                // have no schema we can validate the collapse against, so leave them alone.
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

/// Top-level map-key collapse: when every map key is a concrete object type, every per-key
/// selection set is identical, and the key set exactly matches the runtime objects of some
/// abstract supertype defined in `graph_id`, rewrite the map to a single `{ Abstract:
/// shared_selections }` entry.
///
/// Root operation type entries (`Query` / `Mutation` / `Subscription`) are skipped because
/// they are positional placeholders rather than concrete types reachable through an abstract
/// supertype.
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

/// Recursive walk used by [`collapse_step_selections`].
///
/// At every nested level the walk first strips equal `... on Same { ... }` fragments
/// (where `Same` is the static parent type) and then attempts to fold any group of inline
/// fragments on distinct concrete object types whose selections are identical and whose set
/// covers exactly the runtime objects of an abstract supertype the target subgraph defines,
/// rewriting them into a single `... on Abstract { ... }` fragment.
///
/// `parent_type_name` is the static parent type of `selection_set`. When `None`, the walk
/// still descends through fields and inline fragments using their own type information, but
/// the parent-driven self-flatten and the parent-equals-abstract equal check are skipped.
/// Returns `true` if any rewrite occurred at this level or below.
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

/// Drops inline fragments at this level whose type condition equals `parent_type` and inlines
/// their selections in place, deduplicating against items already present at this level.
/// Fragments with `@skip` / `@include` directives are preserved as-is, since their conditional
/// nature means the contents are not equivalent to an unconditional inlining. Returns `true`
/// when at least one such equal fragment was inlined.
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
            // Two fragments on the same concrete type would have to be merged before we can
            // reason about a single selection per type. Skip this level rather than attempt it.
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

/// Finds an abstract supertype defined in `graph_id` whose runtime objects (interface
/// implementors or union members in that subgraph) exactly equal `concrete_set`, and on which
/// every selected field in `shared_selections` is itself defined.
///
/// When multiple supertypes match (e.g. an interface and a covering union both fit), the
/// lexicographically first name is returned for plan stability.
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
/// `abstract_type` *as seen by the target subgraph `graph_id`*.
///
/// Looking at the supergraph-wide field map would accept a collapse for any field that some
/// subgraph contributes to the abstract type, even when the subgraph the rewrite targets
/// doesn't define that field on the abstract type itself (only on its concrete implementors).
/// In that case the rewritten `... on Abstract { field }` fragment would not validate against
/// the target subgraph's schema. We therefore filter the abstract type's fields through the
/// per-subgraph `@join__field` lens before checking presence.
///
/// Union types never declare fields, so any non-`__typename` selection at this level cannot
/// resolve against a union and the collapse is rejected.
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
                // Inline fragments narrow the type and validate against their own type
                // condition, so they don't need to resolve against the parent abstract type.
                // Fragment spreads similarly carry their own type condition in the operation
                // document and aren't validated against the parent here.
            }
        }
    }

    true
}
