use crate::query_planner::{
    ast::flat::{
        arena::{Arena, ListId, NodeId},
        Kind, Sym, NONE,
    },
    state::supergraph_state::{SupergraphDefinition, SupergraphState},
};

/// Adds the `__typename` the projection needs to the objects in `list`, a root fetch's
/// selections on `type_name`. Not to `list` itself.
pub(super) fn add_typename_for_projection(
    arena: &mut Arena,
    list: ListId,
    type_name: &str,
    supergraph: &SupergraphState,
) {
    let type_name = known_type(supergraph, type_name);
    add_typename(arena, vec![(list, type_name, true)], supergraph);
}

/// Like `add_typename_for_projection`, for the selections of `_entities`, an inline fragment for
/// each entity type.
pub(super) fn add_typename_for_projection_to_entities(
    arena: &mut Arena,
    list: ListId,
    supergraph: &SupergraphState,
) {
    for index in 0..arena.items(list).len() {
        let node = *arena.node(arena.items(list)[index]);
        if node.kind == Kind::InlineFragment {
            let type_name = known_type(supergraph, arena.str(node.name));
            add_typename(arena, vec![(node.children, type_name, true)], supergraph);
        }
    }
}

/// The supergraph's own copy of the name, `None` for a type it doesn't have. Looking the type up
/// is all a name is used for.
fn known_type<'s>(supergraph: &'s SupergraphState, name: &str) -> Option<&'s str> {
    supergraph
        .definitions
        .get_key_value(name)
        .map(|(name, _)| name.as_str())
}

struct Occurrence<'s> {
    field: NodeId,
    response_key: Sym,
    /// The field's type, `None` when it isn't known.
    output_type: Option<&'s str>,
    /// It's there whenever its object is.
    always: bool,
}

/// `sets` are the occurrences of one object: each with the type it's on, `None` when it isn't
/// known, and whether it's there whenever the object is.
fn add_typename<'s>(
    arena: &mut Arena,
    sets: Vec<(ListId, Option<&'s str>, bool)>,
    supergraph: &'s SupergraphState,
) {
    let mut fields = Vec::new();
    for (list, type_name, always) in sets {
        collect(arena, list, type_name, always, supergraph, &mut fields);
    }
    // the occurrences of each object next to each other. Inline fragments don't add a key, the
    // projection merges what's under them into the same object. Interned, so equal keys are
    // equal symbols
    fields.sort_unstable_by_key(|occurrence| occurrence.response_key);
    for occurrences in fields.chunk_by_mut(|a, b| a.response_key == b.response_key) {
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
            let asks = is_abstract && has_typename(arena, arena.node(occurrence.field).children);
            asked |= asks;
            has_typename_always |= asks && occurrence.always;
            any_always |= occurrence.always;
        }
        if asked && !has_typename_always {
            // one that's always there is enough
            for occurrence in occurrences.iter() {
                let children = arena.node(occurrence.field).children;
                if (occurrence.always || !any_always) && !has_typename(arena, children) {
                    let typename = arena.new_field("__typename", None, None, None);
                    let mut items = vec![typename];
                    items.extend_from_slice(arena.items(children));
                    arena.set_items(children, &items);
                }
            }
        }
        let children = occurrences
            .iter()
            .map(|occurrence| {
                let list = arena.node(occurrence.field).children;
                (list, occurrence.output_type, occurrence.always)
            })
            .collect();
        add_typename(arena, children, supergraph);
    }
}

/// The fields of `list` with selections, through its inline fragments. `always` is whether `list`
/// is there whenever its object is, and stays so through fragments with no `@include`/`@skip`.
fn collect<'s>(
    arena: &Arena,
    list: ListId,
    type_name: Option<&str>,
    always: bool,
    supergraph: &'s SupergraphState,
    fields: &mut Vec<Occurrence<'s>>,
) {
    for &id in arena.items(list) {
        let node = arena.node(id);
        let unconditional = node.include_if == NONE && node.skip_if == NONE;
        match node.kind {
            Kind::Field if !arena.is_empty(node.children) => {
                let output_type = type_name
                    .and_then(|name| supergraph.definitions.get(name))
                    .and_then(|definition| definition.fields().get(arena.str(node.name)))
                    .map(|field| field.field_type.inner_type());
                fields.push(Occurrence {
                    field: id,
                    response_key: if node.alias != NONE {
                        node.alias
                    } else {
                        node.name
                    },
                    output_type,
                    always: always && unconditional,
                });
            }
            Kind::InlineFragment => collect(
                arena,
                node.children,
                Some(arena.str(node.name)),
                always && unconditional,
                supergraph,
                fields,
            ),
            Kind::Field | Kind::FragmentSpread => {}
        }
    }
}

/// A `__typename` that's always there, not under a condition.
fn has_typename(arena: &Arena, list: ListId) -> bool {
    arena.items(list).iter().any(|&id| {
        let node = arena.node(id);
        node.kind == Kind::Field
            && arena.str(node.name) == "__typename"
            && node.alias == NONE
            && node.include_if == NONE
            && node.skip_if == NONE
    })
}
