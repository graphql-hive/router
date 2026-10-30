//! The response projection tells the objects of an interface or a union apart by their
//! `__typename`, wherever the client's selections on it sit under type conditions: `results { id }`
//! next to `results @include(if: $a) { ... on Product { id } }` has an `id` for every type and one
//! for `Product`s. The planner asks for `__typename` where a type condition needs it, but that may
//! be under a condition too, here `$a`. With `$a` off the projection can't tell the `Product`s and
//! drops `id`. So such an object gets a `__typename` that's always there.

use ahash::{HashMap, HashSet};

use crate::query_planner::{
    ast::{
        selection_item::SelectionItem,
        selection_set::{FieldSelection, SelectionSet},
    },
    state::supergraph_state::{SupergraphDefinition, SupergraphState},
};

/// Adds `__typename` to the objects of `selection_set` (of type `type_name`) that need it, not to
/// `selection_set` itself.
pub(super) fn add_typename_for_projection(
    selection_set: &mut SelectionSet,
    type_name: &str,
    supergraph: &SupergraphState,
) {
    let mut targets = HashSet::default();
    find_targets(
        &[&*selection_set],
        type_name,
        &mut Vec::new(),
        supergraph,
        &mut targets,
    );
    if !targets.is_empty() {
        add_typename(selection_set, &mut Vec::new(), &targets);
    }
}

/// Like `add_typename_for_projection`, for the selections of `_entities`, an inline fragment for
/// each entity type.
pub(super) fn add_typename_for_projection_to_entities(
    selection_set: &mut SelectionSet,
    supergraph: &SupergraphState,
) {
    let mut by_type: HashMap<&str, Vec<&SelectionSet>> = HashMap::default();
    for item in &selection_set.items {
        if let SelectionItem::InlineFragment(fragment) = item {
            by_type
                .entry(fragment.type_condition.as_str())
                .or_default()
                .push(&fragment.selections);
        }
    }
    let mut targets = HashSet::default();
    for (type_name, sets) in by_type {
        find_targets(&sets, type_name, &mut Vec::new(), supergraph, &mut targets);
    }
    if !targets.is_empty() {
        add_typename(selection_set, &mut Vec::new(), &targets);
    }
}

/// Collects the response key paths of the objects that need `__typename`. `sets` are the
/// selection sets the projection merges into one object of `type_name`: the same response key
/// selected more than once ends up as one value.
fn find_targets(
    sets: &[&SelectionSet],
    type_name: &str,
    path: &mut Vec<String>,
    supergraph: &SupergraphState,
    targets: &mut HashSet<Vec<String>>,
) {
    let mut fields: Vec<(&FieldSelection, &str)> = Vec::new();
    let mut has_fragment = false;
    for set in sets {
        has_fragment |= collect(set, type_name, &mut fields);
    }

    let is_abstract = matches!(
        supergraph.definitions.get(type_name),
        Some(SupergraphDefinition::Interface(_) | SupergraphDefinition::Union(_))
    );
    // the object the planner was asked for can't get one, the top level is `_entities` or a root
    if !path.is_empty() && is_abstract && has_fragment {
        targets.insert(path.clone());
    }

    // order doesn't matter, `targets` is a set
    let mut by_key: HashMap<&str, (Vec<&SelectionSet>, Option<&str>)> = HashMap::default();
    for (field, parent_type) in fields {
        if field.selections.items.is_empty() {
            continue;
        }
        let output_type = supergraph
            .definitions
            .get(parent_type)
            .and_then(|definition| definition.fields().get(&field.name))
            .map(|field| field.field_type.inner_type());
        let (sets, output) = by_key.entry(field.selection_identifier()).or_default();
        sets.push(&field.selections);
        *output = output.or(output_type);
    }
    for (key, (sets, output_type)) in by_key {
        let Some(output_type) = output_type else {
            continue;
        };
        path.push(key.to_string());
        find_targets(&sets, output_type, path, supergraph, targets);
        path.pop();
    }
}

/// The fields of an object through its inline fragments, each with the type it's on. Whether
/// there's an inline fragment at all.
fn collect<'a>(
    set: &'a SelectionSet,
    type_name: &'a str,
    fields: &mut Vec<(&'a FieldSelection, &'a str)>,
) -> bool {
    let mut has_fragment = false;
    for item in &set.items {
        match item {
            SelectionItem::Field(field) => fields.push((field, type_name)),
            SelectionItem::InlineFragment(fragment) => {
                // one on the interface itself gets a scope in the projection too
                has_fragment = true;
                collect(&fragment.selections, &fragment.type_condition, fields);
            }
            SelectionItem::FragmentSpread(_) => {}
        }
    }
    has_fragment
}

fn add_typename(set: &mut SelectionSet, path: &mut Vec<String>, targets: &HashSet<Vec<String>>) {
    for item in &mut set.items {
        match item {
            SelectionItem::Field(field) => {
                if field.selections.items.is_empty() {
                    continue;
                }
                path.push(field.selection_identifier().to_string());
                if targets.contains(path) && !has_typename(&field.selections) {
                    field
                        .selections
                        .items
                        .insert(0, SelectionItem::Field(FieldSelection::new_typename()));
                }
                add_typename(&mut field.selections, path, targets);
                path.pop();
            }
            SelectionItem::InlineFragment(fragment) => {
                add_typename(&mut fragment.selections, path, targets)
            }
            SelectionItem::FragmentSpread(_) => {}
        }
    }
}

/// A `__typename` that's always there, not under a condition.
fn has_typename(set: &SelectionSet) -> bool {
    set.items.iter().any(|item| {
        matches!(item, SelectionItem::Field(field) if field.name == "__typename"
            && field.alias.is_none()
            && field.include_if.is_none()
            && field.skip_if.is_none())
    })
}
