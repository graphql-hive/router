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

    // Compare through `unwrap_self_fragment` so redundant nested `... on Concrete` wrappers
    // don't block same-payload detection.
    let mut effective_iter = entries
        .iter()
        .map(|(k, v)| unwrap_self_fragment(v, k.as_str()));
    let shared = effective_iter.next().expect("len >= 2 checked above");
    for other in effective_iter {
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

/// Recursive walk: at each level folds matching concrete-fragment groups into a single
/// `... on Abstract` fragment, recursing through fields and inline fragments first.
/// `parent_type_name` is the static parent type; when `None` the parent-driven steps are
/// skipped. Returns `true` if anything was rewritten at this level or below.
fn walk_and_collapse(
    selection_set: &mut SelectionSet,
    supergraph: &SupergraphState,
    graph_id: &str,
    parent_type_name: Option<&str>,
) -> bool {
    let mut mutated = false;

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
        let effective = unwrap_self_fragment(&fragment.selections, &fragment.type_condition);
        if concrete_to_selection
            .insert(fragment.type_condition.as_str(), effective)
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
                        // `SelectionItem::PartialEq` ignores alias/args/@skip/@include, so
                        // dedup with it would drop a distinct conditional/aliased copy
                        // (e.g. `masked: username @skip(if: $flag)` vs `username`).
                        if !new_items
                            .iter()
                            .any(|existing| selections_strict_eq(existing, inner))
                            && !is_already_present_later_strict(items, idx, inner, &collapsible_set)
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

/// Strict structural equality that, unlike `SelectionItem::PartialEq`, considers field
/// alias/arguments/`@skip`/`@include` and inline-fragment `@skip`/`@include`. Used only when
/// collapsing concrete fragments into a parent that already names the abstract type, so that
/// truly identical items merge while distinct aliased/conditional copies stay apart.
fn selections_strict_eq(a: &SelectionItem, b: &SelectionItem) -> bool {
    match (a, b) {
        (SelectionItem::Field(x), SelectionItem::Field(y)) => {
            x.name == y.name
                && x.alias == y.alias
                && x.arguments() == y.arguments()
                && x.skip_if == y.skip_if
                && x.include_if == y.include_if
                && selection_sets_strict_eq(&x.selections, &y.selections)
        }
        (SelectionItem::InlineFragment(x), SelectionItem::InlineFragment(y)) => {
            x.type_condition == y.type_condition
                && x.skip_if == y.skip_if
                && x.include_if == y.include_if
                && selection_sets_strict_eq(&x.selections, &y.selections)
        }
        (SelectionItem::FragmentSpread(x), SelectionItem::FragmentSpread(y)) => x == y,
        _ => false,
    }
}

fn selection_sets_strict_eq(a: &SelectionSet, b: &SelectionSet) -> bool {
    a.items.len() == b.items.len()
        && a.items
            .iter()
            .zip(b.items.iter())
            .all(|(x, y)| selections_strict_eq(x, y))
}

fn is_already_present_later_strict(
    items: &[SelectionItem],
    from_idx: usize,
    candidate: &SelectionItem,
    collapsible_set: &BTreeSet<usize>,
) -> bool {
    items
        .iter()
        .enumerate()
        .skip(from_idx + 1)
        .any(|(idx, item)| !collapsible_set.contains(&idx) && selections_strict_eq(item, candidate))
}

/// Strip nested `... on T { ... on T { ... } }` (same `T`, no directives) to the inner selections.
fn unwrap_self_fragment<'a>(set: &'a SelectionSet, type_name: &str) -> &'a SelectionSet {
    if set.items.len() != 1 {
        return set;
    }
    let SelectionItem::InlineFragment(fragment) = &set.items[0] else {
        return set;
    };
    if fragment.type_condition != type_name
        || fragment.include_if.is_some()
        || fragment.skip_if.is_some()
    {
        return set;
    }
    unwrap_self_fragment(&fragment.selections, type_name)
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

#[cfg(test)]
mod unwrap_self_fragment_tests {
    use super::{selections_strict_eq, unwrap_self_fragment};

    use crate::ast::{
        selection_item::SelectionItem,
        selection_set::{FieldSelection, InlineFragmentSelection, SelectionSet},
    };

    fn empty_field(name: impl Into<String>) -> FieldSelection {
        FieldSelection {
            name: name.into(),
            selections: SelectionSet::default(),
            alias: None,
            arguments: None,
            skip_if: None,
            include_if: None,
            omit_from_response: false,
        }
    }

    fn inline_on(type_condition: impl Into<String>, selections: SelectionSet) -> SelectionItem {
        SelectionItem::InlineFragment(InlineFragmentSelection {
            type_condition: type_condition.into(),
            selections,
            skip_if: None,
            include_if: None,
        })
    }

    #[test]
    fn unwrap_peels_redundant_nested_self_fragments() {
        let inner = SelectionSet {
            items: vec![SelectionItem::Field(empty_field("reviewsCount"))],
        };
        let nested = SelectionSet {
            items: vec![inline_on(
                "Book",
                SelectionSet {
                    items: vec![inline_on("Book", inner.clone())],
                },
            )],
        };
        assert_eq!(unwrap_self_fragment(&nested, "Book"), &inner);

        let triple = SelectionSet {
            items: vec![inline_on(
                "Book",
                SelectionSet {
                    items: vec![inline_on("Book", nested)],
                },
            )],
        };
        assert_eq!(unwrap_self_fragment(&triple, "Book"), &inner);
    }

    #[test]
    fn unwrap_stops_on_directived_fragment_even_if_same_type_condition() {
        let leaf = SelectionSet {
            items: vec![SelectionItem::Field(empty_field("id"))],
        };
        let with_skip = SelectionSet {
            items: vec![SelectionItem::InlineFragment(InlineFragmentSelection {
                type_condition: "Book".to_string(),
                selections: leaf.clone(),
                skip_if: Some("hide".into()),
                include_if: None,
            })],
        };
        let outer = SelectionSet {
            items: vec![inline_on("Book", with_skip.clone())],
        };

        assert_eq!(unwrap_self_fragment(&outer, "Book"), &with_skip);
        assert_eq!(unwrap_self_fragment(&with_skip, "Book"), &with_skip);
    }

    #[test]
    fn unwrap_keeps_identity_when_multiple_items_or_mismatched_condition() {
        let two_fields = SelectionSet {
            items: vec![
                SelectionItem::Field(empty_field("id")),
                SelectionItem::Field(empty_field("__typename")),
            ],
        };

        assert_eq!(unwrap_self_fragment(&two_fields, "Book"), &two_fields);

        let wrong_type = SelectionSet {
            items: vec![inline_on("Magazine", SelectionSet::default())],
        };
        assert_eq!(unwrap_self_fragment(&wrong_type, "Book"), &wrong_type);
    }

    fn field_with_alias(name: &str, alias: Option<&str>) -> SelectionItem {
        SelectionItem::Field(FieldSelection {
            name: name.to_string(),
            alias: alias.map(|a| a.to_string()),
            ..empty_field(name)
        })
    }

    fn field_with_skip(name: &str, alias: Option<&str>, skip_if: Option<&str>) -> SelectionItem {
        SelectionItem::Field(FieldSelection {
            name: name.to_string(),
            alias: alias.map(|a| a.to_string()),
            skip_if: skip_if.map(|s| s.to_string()),
            ..empty_field(name)
        })
    }

    #[test]
    fn strict_eq_matches_identical_unaliased_fields() {
        let a = field_with_alias("id", None);
        let b = field_with_alias("id", None);
        assert!(selections_strict_eq(&a, &b));
    }

    #[test]
    fn strict_eq_separates_aliased_from_unaliased_same_name() {
        // The exact case Copilot warned about: shallow `SelectionItem::PartialEq` would treat
        // these as equal because both have name="username" and empty selections.
        let unaliased = field_with_alias("username", None);
        let aliased = field_with_alias("username", Some("masked"));
        assert!(!selections_strict_eq(&unaliased, &aliased));
    }

    #[test]
    fn strict_eq_separates_unconditional_from_skip_guarded_copy() {
        let unconditional = field_with_skip("username", None, None);
        let guarded = field_with_skip("username", None, Some("flag"));
        assert!(!selections_strict_eq(&unconditional, &guarded));
    }

    #[test]
    fn strict_eq_separates_inline_fragments_with_different_directives() {
        let plain = inline_on("Book", SelectionSet::default());
        let with_skip = SelectionItem::InlineFragment(InlineFragmentSelection {
            type_condition: "Book".to_string(),
            selections: SelectionSet::default(),
            skip_if: Some("hide".to_string()),
            include_if: None,
        });
        assert!(!selections_strict_eq(&plain, &with_skip));
    }

    #[test]
    fn strict_eq_recurses_into_nested_selections() {
        let outer_a = SelectionItem::Field(FieldSelection {
            name: "user".to_string(),
            selections: SelectionSet {
                items: vec![field_with_alias("username", None)],
            },
            ..empty_field("user")
        });
        let outer_b = SelectionItem::Field(FieldSelection {
            name: "user".to_string(),
            selections: SelectionSet {
                items: vec![field_with_alias("username", Some("masked"))],
            },
            ..empty_field("user")
        });
        assert!(!selections_strict_eq(&outer_a, &outer_b));
    }
}
