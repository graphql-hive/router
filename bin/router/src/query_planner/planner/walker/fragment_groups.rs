//! Fragments planned once for several object types.
//!
//! Type expansion turns a selection on an interface into a fragment per object type, and a
//! client often writes one per type too: `... on Book { title author } ... on Movie { title
//! director }`. Below each, the planner plans the same thing again: `title` from the same
//! subgraph, the same way. For types it plans the same way, see `Graph::interchangeable_for`,
//! the fields every fragment selects go in one fragment, planned once for all of them, and what's
//! left stays in each type's own: `... on Book { title } ... on Book { author } ... on Movie {
//! director }`, with `Movie` planned with the first `Book`, see `SelectionAttributes::also`.
//!
//! The walker holds on to the operation's selections, so the split is made on a copy of the
//! operation before walking, and the fragments that are planned for more types are found by
//! their address in it.

use std::rc::Rc;

use ahash::HashMap;

use crate::query_planner::{
    ast::{
        operation::OperationDefinition,
        selection_item::SelectionItem,
        selection_set::{FieldSelection, InlineFragmentSelection, SelectionSet},
    },
    graph::Graph,
    planner::walker::path::SelectionAttributes,
    state::supergraph_state::{OperationKind, SupergraphDefinition, SupergraphState},
};

/// The fragments of an operation planned for more types than their own, by address.
#[derive(Default)]
pub struct FragmentGroups(HashMap<usize, Rc<SelectionAttributes>>);

impl FragmentGroups {
    /// The other types `item` is planned for, when it's such a fragment.
    pub(crate) fn of(&self, item: &SelectionItem) -> Option<Rc<SelectionAttributes>> {
        if self.0.is_empty() {
            return None;
        }
        match item {
            SelectionItem::InlineFragment(fragment) => self
                .0
                .get(&(fragment as *const InlineFragmentSelection as usize))
                .cloned(),
            _ => None,
        }
    }
}

/// `operation` with the fragments split, and which of them are planned for more types. `None`
/// when nothing gets split.
pub fn group_fragments(
    graph: &Graph<'_>,
    supergraph: &SupergraphState,
    operation: &OperationDefinition,
) -> Option<(OperationDefinition, FragmentGroups)> {
    if graph.has_context_arguments()
        || (graph.type_groups.is_empty() && graph.type_profiles.is_empty())
    {
        return None;
    }
    let root = supergraph
        .root_type_name(Some(
            operation
                .operation_kind
                .as_ref()
                .unwrap_or(&OperationKind::Query),
        ))
        .ok()?;
    let mut grouped = Vec::new();
    let selection_set = Splitter { graph, supergraph }.selection_set(
        &operation.selection_set,
        root,
        &[],
        &mut Vec::new(),
        &mut grouped,
    )?;
    let operation = OperationDefinition {
        name: operation.name.clone(),
        operation_kind: operation.operation_kind.clone(),
        selection_set,
        variable_definitions: operation.variable_definitions.clone(),
    };
    // the items don't move anymore, so their addresses are what the walker sees
    let mut groups = HashMap::default();
    for (position, also) in grouped {
        let mut selection_set = &operation.selection_set;
        let mut item = None;
        for &index in &position {
            let current = &selection_set.items[index];
            item = Some(current);
            selection_set = match current {
                SelectionItem::Field(field) => &field.selections,
                SelectionItem::InlineFragment(fragment) => &fragment.selections,
                SelectionItem::FragmentSpread(_) => selection_set,
            };
        }
        if let Some(SelectionItem::InlineFragment(fragment)) = item {
            groups.insert(
                fragment as *const InlineFragmentSelection as usize,
                Rc::new(SelectionAttributes {
                    alias: None,
                    arguments: None,
                    also: Some(also),
                }),
            );
        }
    }
    Some((operation, FragmentGroups(groups)))
}

struct Splitter<'a> {
    graph: &'a Graph<'a>,
    supergraph: &'a SupergraphState,
}

/// An item of a selection set being split: one of the operation's, or a fragment made for it.
enum Part<'s> {
    Kept(&'s SelectionItem),
    Made(SelectionItem),
}

impl Part<'_> {
    fn item(&self) -> &SelectionItem {
        match self {
            Part::Kept(item) => item,
            Part::Made(item) => item,
        }
    }
}

/// `item` with `selections` in place of its own.
fn with_selections(item: &SelectionItem, selections: SelectionSet) -> SelectionItem {
    match item {
        SelectionItem::Field(field) => SelectionItem::Field(FieldSelection {
            name: field.name.clone(),
            alias: field.alias.clone(),
            selections,
            arguments: field.arguments.clone(),
            skip_if: field.skip_if.clone(),
            include_if: field.include_if.clone(),
            omit_from_response: field.omit_from_response,
        }),
        SelectionItem::InlineFragment(fragment) => {
            SelectionItem::InlineFragment(InlineFragmentSelection {
                type_condition: fragment.type_condition.clone(),
                selections,
                skip_if: fragment.skip_if.clone(),
                include_if: fragment.include_if.clone(),
            })
        }
        SelectionItem::FragmentSpread(_) => item.clone(),
    }
}

/// Fragments that go together: the first one's type, and the items all of them select.
struct Cluster<'s> {
    members: Vec<usize>,
    shared: Vec<&'s SelectionItem>,
}

impl<'a> Splitter<'a> {
    /// `selection_set` on `type_name`, with its fragments split, `None` when nothing in it is.
    /// The objects come from `subgraphs`. `position` is where it is, by item indexes, and
    /// `grouped` gets where each fragment planned for more types ends up.
    fn selection_set(
        &self,
        selection_set: &SelectionSet,
        type_name: &str,
        subgraphs: &[&str],
        position: &mut Vec<usize>,
        grouped: &mut Vec<(Vec<usize>, Rc<[String]>)>,
    ) -> Option<SelectionSet> {
        let clusters = self.clusters(selection_set, subgraphs);
        let mut parts = Vec::with_capacity(selection_set.items.len());
        let mut new_groups = Vec::new();
        for (index, item) in selection_set.items.iter().enumerate() {
            let cluster = clusters
                .iter()
                .find(|cluster| cluster.members.contains(&index));
            match (item, cluster) {
                (SelectionItem::InlineFragment(fragment), Some(cluster)) => {
                    if cluster.members[0] == index {
                        let also: Rc<[String]> = cluster.members[1..]
                            .iter()
                            .filter_map(|&member| match &selection_set.items[member] {
                                SelectionItem::InlineFragment(other) => {
                                    Some(other.type_condition.clone())
                                }
                                _ => None,
                            })
                            .collect();
                        new_groups.push((parts.len(), also));
                        parts.push(Part::Made(SelectionItem::InlineFragment(
                            InlineFragmentSelection {
                                type_condition: fragment.type_condition.clone(),
                                selections: SelectionSet {
                                    items: cluster
                                        .shared
                                        .iter()
                                        .map(|item| (*item).clone())
                                        .collect(),
                                },
                                skip_if: fragment.skip_if.clone(),
                                include_if: fragment.include_if.clone(),
                            },
                        )));
                    }
                    let rest: Vec<SelectionItem> = fragment
                        .selections
                        .items
                        .iter()
                        .filter(|item| !cluster.shared.iter().any(|shared| same_item(shared, item)))
                        .cloned()
                        .collect();
                    if !rest.is_empty() {
                        parts.push(Part::Made(SelectionItem::InlineFragment(
                            InlineFragmentSelection {
                                type_condition: fragment.type_condition.clone(),
                                selections: SelectionSet { items: rest },
                                skip_if: fragment.skip_if.clone(),
                                include_if: fragment.include_if.clone(),
                            },
                        )));
                    }
                }
                (item, _) => parts.push(Part::Kept(item)),
            }
        }
        // then below every item, before copying any: an item with nothing split below it is
        // copied once, by the first level that changes
        let mut changed = !clusters.is_empty();
        let mut below = Vec::with_capacity(parts.len());
        for (index, part) in parts.iter().enumerate() {
            position.push(index);
            let selections = self.below(part.item(), type_name, subgraphs, position, grouped);
            position.pop();
            changed |= selections.is_some();
            below.push(selections);
        }
        for (index, also) in new_groups {
            let mut at = position.clone();
            at.push(index);
            grouped.push((at, also));
        }
        if !changed {
            return None;
        }
        let items = parts
            .into_iter()
            .zip(below)
            .map(|(part, below)| match (part, below) {
                (Part::Kept(item), None) => item.clone(),
                (Part::Kept(item), Some(selections)) => with_selections(item, selections),
                (Part::Made(item), Some(selections)) => with_selections(&item, selections),
                (Part::Made(item), None) => item,
            })
            .collect();
        Some(SelectionSet { items })
    }

    /// The selections of `item` on `type_name`, with their fragments split, `None` when
    /// nothing in them is.
    fn below(
        &self,
        item: &SelectionItem,
        type_name: &str,
        subgraphs: &[&str],
        position: &mut Vec<usize>,
        grouped: &mut Vec<(Vec<usize>, Rc<[String]>)>,
    ) -> Option<SelectionSet> {
        match item {
            SelectionItem::Field(field) => {
                let return_type = self
                    .supergraph
                    .field_return_type_name(type_name, &field.name)?;
                let subgraphs = self.resolving(type_name, field);
                self.selection_set(
                    &field.selections,
                    return_type,
                    &subgraphs,
                    position,
                    grouped,
                )
            }
            SelectionItem::InlineFragment(fragment) => self.selection_set(
                &fragment.selections,
                &fragment.type_condition,
                subgraphs,
                position,
                grouped,
            ),
            SelectionItem::FragmentSpread(_) => None,
        }
    }

    /// The subgraphs that resolve `field` on `type_name`, where the objects it returns are.
    fn resolving(&self, type_name: &str, field: &FieldSelection) -> Vec<&'a str> {
        let Some(definition) = self.supergraph.definitions.get(type_name) else {
            return Vec::new();
        };
        let joins = definition
            .fields()
            .get(&field.name)
            .map(|field| field.join_field.as_slice())
            .unwrap_or_default();
        let mut subgraphs: Vec<&str> = if joins.is_empty() {
            match definition {
                SupergraphDefinition::Object(object) => object
                    .join_type
                    .iter()
                    .map(|join| join.graph_id.as_str())
                    .collect(),
                SupergraphDefinition::Interface(interface) => interface
                    .join_type
                    .iter()
                    .map(|join| join.graph_id.as_str())
                    .collect(),
                _ => Vec::new(),
            }
        } else {
            joins
                .iter()
                .filter(|join| !join.external)
                .filter_map(|join| join.graph_id.as_deref())
                .collect()
        };
        subgraphs.sort_unstable();
        subgraphs.dedup();
        subgraphs
    }

    /// The fragments of `selection_set` planned together, each with what they all select.
    fn clusters<'s>(
        &self,
        selection_set: &'s SelectionSet,
        subgraphs: &[&str],
    ) -> Vec<Cluster<'s>> {
        let fragments: Vec<(usize, &InlineFragmentSelection)> = selection_set
            .items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| match item {
                SelectionItem::InlineFragment(fragment)
                    if self
                        .graph
                        .type_groups
                        .contains_key(fragment.type_condition.as_str())
                        || self
                            .graph
                            .type_profiles
                            .contains_key(fragment.type_condition.as_str()) =>
                {
                    Some((index, fragment))
                }
                _ => None,
            })
            .collect();
        if fragments.len() < 2 || subgraphs.is_empty() {
            return Vec::new();
        }
        let mut clusters: Vec<Cluster<'s>> = Vec::new();
        for &(index, fragment) in &fragments {
            let mut joined = false;
            for cluster in clusters.iter_mut() {
                let SelectionItem::InlineFragment(first) = &selection_set.items[cluster.members[0]]
                else {
                    continue;
                };
                if first.skip_if != fragment.skip_if || first.include_if != fragment.include_if {
                    continue;
                }
                let shared: Vec<&'s SelectionItem> = cluster
                    .shared
                    .iter()
                    .copied()
                    .filter(|item| {
                        fragment
                            .selections
                            .items
                            .iter()
                            .any(|theirs| same_item(item, theirs))
                    })
                    .filter(|item| !self.per_type(&fragment.type_condition, item))
                    .collect();
                if shared.is_empty()
                    || !self.graph.interchangeable_for(
                        &first.type_condition,
                        &fragment.type_condition,
                        self.with_requirements(
                            &first.type_condition,
                            shared.iter().filter_map(|item| match item {
                                SelectionItem::Field(field) => Some(field.name.as_str()),
                                _ => None,
                            }),
                        )
                        .into_iter(),
                        subgraphs.iter().copied(),
                    )
                {
                    continue;
                }
                cluster.shared = shared;
                cluster.members.push(index);
                joined = true;
                break;
            }
            if !joined {
                clusters.push(Cluster {
                    members: vec![index],
                    shared: fragment
                        .selections
                        .items
                        .iter()
                        .filter(|item| matches!(item, SelectionItem::Field(_)))
                        .filter(|item| !self.per_type(&fragment.type_condition, item))
                        .collect(),
                });
            }
        }
        clusters.retain(|cluster| cluster.members.len() > 1 && !cluster.shared.is_empty());
        clusters
    }

    /// Whether `item` is planned per type: not a field of `type_name`, or one with a
    /// `@requires` that isn't a plain list of fields.
    fn per_type(&self, type_name: &str, item: &SelectionItem) -> bool {
        let SelectionItem::Field(field) = item else {
            return true;
        };
        self.supergraph
            .definitions
            .get(type_name)
            .and_then(|definition| definition.fields().get(&field.name))
            .is_none_or(|field| {
                field.join_field.iter().any(|join| {
                    join.requires
                        .as_deref()
                        .is_some_and(|requires| !plain_fields(requires))
                })
            })
    }

    /// `fields` of `type_name`, and the fields their `@requires` need, all the way down. The
    /// types plan a field with `@requires` the same way when they plan these the same way.
    fn with_requirements<'f>(
        &self,
        type_name: &str,
        fields: impl Iterator<Item = &'f str>,
    ) -> Vec<&'f str>
    where
        'a: 'f,
    {
        let mut all: Vec<&'f str> = fields.collect();
        let Some(definition) = self.supergraph.definitions.get(type_name) else {
            return all;
        };
        let mut next = 0;
        while next < all.len() {
            let name = all[next];
            next += 1;
            let Some(field) = definition.fields().get(name) else {
                continue;
            };
            for join in &field.join_field {
                for required in join
                    .requires
                    .iter()
                    .flat_map(|requires| requires.split_whitespace())
                {
                    if !all.contains(&required) {
                        all.push(required);
                    }
                }
            }
        }
        all
    }
}

/// Whether a `@requires` selection is only field names, like `"price currency"`.
fn plain_fields(selection: &str) -> bool {
    selection
        .chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c.is_whitespace())
}

/// Whether two items select exactly the same, all the way down.
fn same_item(a: &SelectionItem, b: &SelectionItem) -> bool {
    match (a, b) {
        (SelectionItem::Field(a), SelectionItem::Field(b)) => {
            a.name == b.name
                && a.alias == b.alias
                && a.arguments() == b.arguments()
                && a.skip_if == b.skip_if
                && a.include_if == b.include_if
                && same_selections(&a.selections, &b.selections)
        }
        (SelectionItem::InlineFragment(a), SelectionItem::InlineFragment(b)) => {
            a.type_condition == b.type_condition
                && a.skip_if == b.skip_if
                && a.include_if == b.include_if
                && same_selections(&a.selections, &b.selections)
        }
        (SelectionItem::FragmentSpread(a), SelectionItem::FragmentSpread(b)) => a == b,
        _ => false,
    }
}

fn same_selections(a: &SelectionSet, b: &SelectionSet) -> bool {
    a.items.len() == b.items.len() && a.items.iter().zip(&b.items).all(|(a, b)| same_item(a, b))
}
