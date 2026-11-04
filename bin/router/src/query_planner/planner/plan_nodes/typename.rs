use crate::query_planner::{
    ast::{
        selection_item::SelectionItem,
        selection_set::{FieldSelection, SelectionSet},
    },
    state::supergraph_state::{SupergraphDefinition, SupergraphState},
};

/// `selection_set` is a root fetch's selections, of type `type_name`.
pub(super) fn add_typename_for_projection(
    selection_set: &mut SelectionSet,
    type_name: &str,
    supergraph: &SupergraphState,
) {
    add_typename(vec![(selection_set, Some(type_name), true)], supergraph);
}

/// Like `add_typename_for_projection`, for the selections of `_entities`, an inline fragment for
/// each entity type.
pub(super) fn add_typename_for_projection_to_entities(
    selection_set: &mut SelectionSet,
    supergraph: &SupergraphState,
) {
    for item in &mut selection_set.items {
        if let SelectionItem::InlineFragment(fragment) = item {
            let set = (
                &mut fragment.selections,
                Some(&*fragment.type_condition),
                true,
            );
            add_typename(vec![set], supergraph);
        }
    }
}

struct Occurrence<'a, 's> {
    field: &'a mut FieldSelection,
    /// The field's type, `None` when it isn't known.
    output_type: Option<&'s str>,
    /// It's there whenever its object is.
    always: bool,
}

/// `sets` are the occurrences of one object: each with the type it's on, `None` when it isn't
/// known, and whether it's there whenever the object is.
fn add_typename(sets: Vec<(&mut SelectionSet, Option<&str>, bool)>, supergraph: &SupergraphState) {
    let mut fields = Vec::new();
    for (set, type_name, always) in sets {
        collect(set, type_name, always, supergraph, &mut fields);
    }
    // the occurrences of each object next to each other. Inline fragments don't add a key, the
    // projection merges what's under them into the same object
    fields.sort_unstable_by(|a, b| {
        a.field
            .selection_identifier()
            .cmp(b.field.selection_identifier())
    });
    for occurrences in
        fields.chunk_by_mut(|a, b| a.field.selection_identifier() == b.field.selection_identifier())
    {
        let only = occurrences.len() == 1;
        let (mut asked, mut has_typename_always, mut any_always) = (false, false, false);
        for occurrence in occurrences.iter_mut() {
            occurrence.always |= only;
            // one we can't tell counts as abstract, a `__typename` too many beats a field
            // missing from the response
            let is_abstract = occurrence.output_type.is_none_or(|name| {
                matches!(
                    supergraph.definitions.get(name),
                    Some(SupergraphDefinition::Interface(_) | SupergraphDefinition::Union(_))
                )
            });
            let asks = is_abstract && has_typename(&occurrence.field.selections);
            asked |= asks;
            has_typename_always |= asks && occurrence.always;
            any_always |= occurrence.always;
        }
        if asked && !has_typename_always {
            // one that's always there is enough
            for occurrence in occurrences.iter_mut() {
                if (occurrence.always || !any_always) && !has_typename(&occurrence.field.selections)
                {
                    let typename = SelectionItem::Field(FieldSelection::new_typename());
                    occurrence.field.selections.items.insert(0, typename);
                }
            }
        }
        let children = occurrences
            .iter_mut()
            .map(|occurrence| {
                let set = &mut occurrence.field.selections;
                (set, occurrence.output_type, occurrence.always)
            })
            .collect();
        add_typename(children, supergraph);
    }
}

/// The fields of `set` with selections, through its inline fragments. `always` is whether `set` is
/// there whenever its object is, and stays so through fragments with no `@include`/`@skip`.
fn collect<'a, 's>(
    set: &'a mut SelectionSet,
    type_name: Option<&str>,
    always: bool,
    supergraph: &'s SupergraphState,
    fields: &mut Vec<Occurrence<'a, 's>>,
) {
    for item in &mut set.items {
        match item {
            SelectionItem::Field(field) if !field.selections.items.is_empty() => {
                let output_type = type_name
                    .and_then(|name| supergraph.definitions.get(name))
                    .and_then(|definition| definition.fields().get(&field.name))
                    .map(|field| field.field_type.inner_type());
                let always = always && is_unconditional(&field.include_if, &field.skip_if);
                fields.push(Occurrence {
                    field,
                    output_type,
                    always,
                });
            }
            SelectionItem::InlineFragment(fragment) => {
                let always = always && is_unconditional(&fragment.include_if, &fragment.skip_if);
                let type_name = Some(&*fragment.type_condition);
                collect(
                    &mut fragment.selections,
                    type_name,
                    always,
                    supergraph,
                    fields,
                );
            }
            SelectionItem::Field(_) | SelectionItem::FragmentSpread(_) => {}
        }
    }
}

fn is_unconditional(include_if: &Option<String>, skip_if: &Option<String>) -> bool {
    include_if.is_none() && skip_if.is_none()
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
