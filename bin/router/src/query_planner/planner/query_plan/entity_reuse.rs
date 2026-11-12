//! Finds where an entity fetch could take its data from the response instead of the subgraph:
//! every position an earlier fetch filled with the same type and (at least) the same fields.
//!
//! Fields are matched by name, arguments and response key, not by subgraph.
//! A field resolved by two subgraphs is `@shareable`, which promises the same data.
//!
//! Only in queries. Entity calls are queries even in a mutation, but there each root field can
//! change an entity an earlier one returned, so what's in the response may be stale.

use std::{hash::Hasher, sync::Arc};

use ahash::{AHasher, HashMap};

use crate::query_planner::{
    ast::{
        requires::RequiresSelectionSet,
        selection_item::SelectionItem,
        selection_set::{FieldSelection, InlineFragmentSelection, SelectionSet},
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

type Step = FetchStepData<MultiTypeFetchStep>;

/// The positions earlier waves filled, by type. Each step is walked once, when its wave is done,
/// and only types some entity fetch of the plan could copy are kept, so a plan without one
/// walks nothing.
pub(super) struct EntityReuseIndex<'a> {
    supergraph: &'a SupergraphState,
    positions: HashMap<&'a str, Positions<'a>>,
    /// what `find` answered since the last wave, by type and key masks. Fetches in one wave
    /// often ask for the same thing, and the positions don't change until the wave is done.
    found: HashMap<(&'a str, u64, u64), Vec<Found<'a>>>,
}

struct Found<'a> {
    needed: &'a SelectionSet,
    representation: &'a SelectionSet,
    reuse: Option<Arc<EntityReuse>>,
}

#[derive(Default)]
struct Positions<'a> {
    /// every distinct path, so `find` compares ids instead of paths
    paths: Vec<FlattenNodePath>,
    /// path ids by the hash of the path. A collision falls back to a scan of `paths`.
    ids: HashMap<u64, usize>,
    list: Vec<Position<'a>>,
}

struct Position<'a> {
    path: usize,
    selections: &'a SelectionSet,
    /// see [`key_mask`]
    keys: u64,
}

/// A path segment that borrows its name, so walking doesn't allocate
#[derive(Clone, Copy)]
enum Segment<'a> {
    Field(&'a str),
    List,
    TypeCondition(&'a str),
}

impl Segment<'_> {
    fn to_path_segment(self) -> PathSegment {
        match self {
            Segment::Field(name) => PathSegment::Field(name.into()),
            Segment::List => PathSegment::List,
            Segment::TypeCondition(name) => {
                PathSegment::TypeCondition(Box::new(TypeCondition::from_names([name])))
            }
        }
    }

    /// Hashes like [`hash_path_segment`] does, so a path hashes the same no matter which part of
    /// it came from the step's response path
    fn hash(self, hasher: &mut AHasher) {
        match self {
            Segment::Field(name) => hash_name(hasher, 0, name),
            Segment::List => hasher.write_u8(1),
            Segment::TypeCondition(name) => hash_name(hasher, 2, name),
        }
    }

    fn matches(self, segment: &PathSegment) -> bool {
        match (self, segment) {
            (Segment::Field(name), PathSegment::Field(other)) => name == &**other,
            (Segment::List, PathSegment::List) => true,
            (Segment::TypeCondition(name), PathSegment::TypeCondition(other)) => {
                matches!(other.names(), [only] if &**only == name)
            }
            _ => false,
        }
    }
}

fn hash_path_segment(hasher: &mut AHasher, segment: &PathSegment) {
    match segment {
        PathSegment::Field(name) => hash_name(hasher, 0, name),
        PathSegment::List => hasher.write_u8(1),
        PathSegment::TypeCondition(condition) => {
            for name in condition.names() {
                hash_name(hasher, 2, name);
            }
        }
    }
}

fn hash_name(hasher: &mut AHasher, tag: u8, name: &str) {
    hasher.write_u8(tag);
    hasher.write(name.as_bytes());
    hasher.write_u8(0xff);
}

/// Where a step's walk starts
struct Start {
    path: FlattenNodePath,
    /// the hash of `path`, to continue with the rest of a position's path
    hasher: AHasher,
}

impl Start {
    fn new(path: FlattenNodePath) -> Self {
        let mut hasher = AHasher::default();
        for segment in path.as_slice() {
            hash_path_segment(&mut hasher, segment);
        }
        Self { path, hasher }
    }
}

impl<'a> EntityReuseIndex<'a> {
    pub(super) fn new(
        fetch_graph: &'a FetchGraph<MultiTypeFetchStep>,
        supergraph: &'a SupergraphState,
    ) -> Self {
        let mut positions = HashMap::default();
        if fetch_graph.operation_kind == OperationKind::Query {
            for index in fetch_graph.step_indices() {
                if let Some((type_name, _)) = fetch_graph.get_step_data(index).ok().and_then(wanted)
                {
                    positions.entry(type_name).or_default();
                }
            }
        }
        Self {
            supergraph,
            positions,
            found: HashMap::default(),
        }
    }

    /// Adds the positions `step` fills. Call it once the step's wave is planned.
    pub(super) fn add_step(&mut self, step: &'a Step) {
        if self.positions.is_empty() || has_context(step) {
            return;
        }
        self.found.clear();
        let start = Start::new(FlattenNodePath::from(&step.response_path));
        let mut path = Vec::new();
        for (definition_name, selections) in step.output.iter_selections() {
            self.walk(&start, &mut path, selections, definition_name);
        }
    }

    /// Tells the entity fetch in `node` where earlier waves already put what it asks for.
    pub(super) fn attach(&mut self, node: &mut PlanNode, step: &'a Step) {
        let PlanNode::Flatten(flatten) = node else {
            return;
        };
        let PlanNode::Fetch(fetch) = flatten.node.as_mut() else {
            return;
        };
        fetch.entity_reuse = self.find(step);
    }

    fn find(&mut self, step: &'a Step) -> Option<Arc<EntityReuse>> {
        let (type_name, needed) = wanted(step)?;
        let positions = self.positions.get_mut(type_name)?;
        let representation = step.input.selections_for_definition(type_name)?;
        let needed_keys = key_mask(needed, |field| field.name != "__typename");
        let representation_keys = key_mask(representation, |_| true);
        let found = self
            .found
            .entry((type_name, needed_keys, representation_keys))
            .or_default();
        if let Some(found) = found
            .iter()
            .find(|found| found.needed == needed && found.representation == representation)
        {
            return found.reuse.clone();
        }
        let reuse = find_in(
            positions,
            type_name,
            needed,
            needed_keys,
            representation,
            representation_keys,
        )
        .map(Arc::new);
        found.push(Found {
            needed,
            representation,
            reuse: reuse.clone(),
        });
        reuse
    }

    fn walk(
        &mut self,
        start: &Start,
        path: &mut Vec<Segment<'a>>,
        selections: &'a SelectionSet,
        current_type: &'a str,
    ) {
        if let Some(positions) = self.positions.get_mut(current_type) {
            positions.add(start, path, selections);
        }

        for item in &selections.items {
            let depth = path.len();
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
                    path.push(Segment::Field(
                        field.alias.as_deref().unwrap_or(&field.name),
                    ));
                    for _ in 0..list_depth(field_type) {
                        path.push(Segment::List);
                    }
                    self.walk(start, path, &field.selections, field_type.inner_type());
                }
                SelectionItem::InlineFragment(fragment) => {
                    if fragment.type_condition != current_type {
                        path.push(Segment::TypeCondition(&fragment.type_condition));
                    }
                    self.walk(start, path, &fragment.selections, &fragment.type_condition);
                }
                _ => {}
            }
            path.truncate(depth);
        }
    }
}

impl<'a> Positions<'a> {
    fn add(&mut self, start: &Start, path: &[Segment<'_>], selections: &'a SelectionSet) {
        let mut hasher = start.hasher.clone();
        for segment in path {
            segment.hash(&mut hasher);
        }
        let hash = hasher.finish();
        let is_path = |known: &FlattenNodePath| {
            let (head, tail) = known
                .as_slice()
                .split_at_checked(start.path.as_slice().len())
                .unwrap_or_default();
            head == start.path.as_slice()
                && tail.len() == path.len()
                && path
                    .iter()
                    .zip(tail)
                    .all(|(segment, known)| segment.matches(known))
        };
        let id = match self.ids.get(&hash) {
            Some(&id) if is_path(&self.paths[id]) => id,
            // another path with the same hash, or a new one
            known => match known.and_then(|_| self.paths.iter().position(is_path)) {
                Some(id) => id,
                None => {
                    let full: Vec<PathSegment> = start
                        .path
                        .as_slice()
                        .iter()
                        .cloned()
                        .chain(path.iter().map(|segment| segment.to_path_segment()))
                        .collect();
                    self.paths.push(full.into());
                    let id = self.paths.len() - 1;
                    self.ids.entry(hash).or_insert(id);
                    id
                }
            },
        };
        self.list.push(Position {
            path: id,
            selections,
            keys: key_mask(selections, |_| true),
        });
    }
}

fn find_in(
    positions: &Positions<'_>,
    type_name: &str,
    needed: &SelectionSet,
    needed_keys: u64,
    representation: &SelectionSet,
    representation_keys: u64,
) -> Option<EntityReuse> {
    // a plan can have thousands of positions, so the masks rule most of them out before the
    // real checks
    let mut is_source = vec![false; positions.paths.len()];
    let mut sources = Vec::new();
    for position in &positions.list {
        if position.keys & needed_keys == needed_keys
            && !is_source[position.path]
            && covers(position.selections, needed)
        {
            is_source[position.path] = true;
            sources.push(position.path);
        }
    }
    if sources.is_empty() {
        return None;
    }
    // the executor matches entities by their representation, read from these positions. a user
    // alias like `id: username` would match the wrong entity there, so a position that puts
    // another field under a key of the representation rules out its path.
    for position in &positions.list {
        if position.keys & representation_keys != 0
            && is_source[position.path]
            && conflicts_with(position.selections, representation)
        {
            is_source[position.path] = false;
        }
    }
    sources.retain(|&path| is_source[path]);
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
        sources: sources
            .into_iter()
            .map(|path| positions.paths[path].clone())
            .collect(),
        copy: RequiresSelectionSet::from(&copy),
    })
}

/// The type and fields an entity fetch could copy from the response, if it could copy at all
fn wanted(step: &Step) -> Option<(&str, &SelectionSet)> {
    if !step.is_entity_call()
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
    is_plain(needed).then_some((type_name, needed))
}

/// One bit per response key of the fields in `selections` that pass `keep`. A position can only
/// cover a selection if it has all of its bits, and only conflict with one if they share a bit.
fn key_mask(selections: &SelectionSet, keep: impl Fn(&FieldSelection) -> bool) -> u64 {
    selections.items.iter().fold(0, |mask, item| match item {
        SelectionItem::Field(field) if keep(field) => {
            let key = field.alias.as_deref().unwrap_or(&field.name);
            let hash = key.bytes().fold(0u32, |hash, byte| {
                hash.wrapping_mul(31).wrapping_add(byte as u32)
            });
            mask | 1 << (hash % 64)
        }
        _ => mask,
    })
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
fn has_context(step: &Step) -> bool {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_gets_one_id_wherever_its_step_starts() {
        let selections = SelectionSet::default();
        let mut positions = Positions::default();
        let top = || PathSegment::Field("top".into());
        let start = |path: Vec<PathSegment>| Start::new(FlattenNodePath::from(path));

        positions.add(&start(vec![top()]), &[Segment::List], &selections);
        positions.add(&start(vec![top(), PathSegment::List]), &[], &selections);
        positions.add(
            &start(vec![]),
            &[Segment::Field("top"), Segment::List],
            &selections,
        );
        positions.add(&start(vec![top()]), &[], &selections);

        let ids: Vec<usize> = positions
            .list
            .iter()
            .map(|position| position.path)
            .collect();
        assert_eq!(ids, [0, 0, 0, 1]);
        assert_eq!(positions.paths.len(), 2);
    }
}
