//! Finds where an entity fetch could take its data from the response instead of the subgraph:
//! every position filled with the same type and (at least) the same fields by a fetch it depends
//! on, directly or through other fetches. A fetch runs once those are done, so only they are sure
//! to be in the response by then, another one might still be on its way.
//!
//! The one exception: an entity fetch that asks a subgraph what an earlier one asked it
//! (the same field deeper in the query, say) waits for that one too, if that lets it copy more.
//! Only when the earlier one waits for nothing this one doesn't already: then it's ready no later
//! than this one, the wait is at most the rest of that one call, and it saves a subgraph call.
//! An earlier planner wave alone doesn't promise that, the earlier one could be held up by a slow
//! chain this one never waits for. [`WAIT_FOR_SAME_FETCH`] turns it off.
//!
//! Fields are matched by name, arguments and response key, not by subgraph.
//! A field resolved by two subgraphs is `@shareable`, which promises the same data.
//!
//! Only in queries. Entity calls are queries even in a mutation, but there each root field can
//! change an entity an earlier one returned, so what's in the response may be stale.

use std::{hash::Hasher, sync::Arc};

use ahash::{AHasher, HashMap};
use petgraph::{
    graph::NodeIndex,
    visit::{EdgeRef, NodeIndexable},
};

use crate::query_planner::{
    ast::{
        merge_path::MergePath,
        requires::RequiresSelectionSet,
        selection_item::SelectionItem,
        selection_set::{
            fields_through_fragments, response_keys_conflict, FieldSelection,
            InlineFragmentSelection, SelectionSet,
        },
    },
    planner::{
        fetch::{
            fetch_graph::FetchGraph,
            fetch_step_data::{FetchStepData, FetchStepFlags},
            state::MultiTypeFetchStep,
        },
        plan_nodes::{
            planning::PlanNode, EntityReuse, FetchId, FetchIdList, FlattenNodePath, PathSegment,
            TypeCondition,
        },
    },
    state::supergraph_state::{OperationKind, SupergraphField, SupergraphState, TypeNode},
};

type Step = FetchStepData<MultiTypeFetchStep>;

/// Whether an entity fetch may wait for the same fetch earlier on, see the module docs. It trades
/// a subgraph call for a wait, which is usually short but not always, so it can be turned off
/// here. Not a config option (yet).
const WAIT_FOR_SAME_FETCH: bool = true;

/// The positions earlier waves filled, by type. Each step is walked once, when its wave is done,
/// and only types some entity fetch of the plan could copy are kept, so a plan without one
/// walks nothing. A fetch's dependencies are always in earlier waves.
pub(super) struct EntityReuseIndex<'a> {
    fetch_graph: &'a FetchGraph<MultiTypeFetchStep>,
    supergraph: &'a SupergraphState,
    positions: HashMap<&'a str, Positions<'a>>,
    /// the subgraph of each step that filled a position, by step index, see [`resolved_by`]
    subgraphs: Vec<u16>,
    /// what `find` answered since the last wave, by type and key masks. Fetches in one wave
    /// often ask for the same thing, and the positions don't change until the wave is done.
    found: HashMap<(&'a str, KeyMask, KeyMask), Vec<Found<'a>>>,
    /// the entity fetches of earlier waves that could copy, a later fetch asking the same may
    /// wait for one of them
    earlier_fetches: Vec<EarlierEntityFetch<'a>>,
}

struct EarlierEntityFetch<'a> {
    index: NodeIndex,
    id: FetchId,
    service: &'a str,
    type_name: &'a str,
    needed: &'a SelectionSet,
    /// what it waits for, a later fetch only waits for it when it waits for all of that anyway
    depends_on: Dependencies,
}

/// What an entity fetch would copy: its type and fields, and the representation the executor
/// matches entities by. The masks are theirs, see [`key_mask`].
#[derive(Clone, Copy)]
struct Wanted<'a> {
    type_name: &'a str,
    needed: &'a SelectionSet,
    representation: &'a SelectionSet,
    needed_keys: KeyMask,
    representation_keys: KeyMask,
    /// the subgraph the fetch goes to, see `find_in`
    subgraph: Option<u16>,
}

impl<'a> Wanted<'a> {
    fn new(
        type_name: &'a str,
        needed: &'a SelectionSet,
        representation: &'a SelectionSet,
        subgraph: Option<u16>,
    ) -> Self {
        Self {
            subgraph,
            type_name,
            needed,
            representation,
            // `covers` counts `__typename` as there, the executor checks for it before copying
            needed_keys: key_mask(needed, |field| field.name != "__typename"),
            representation_keys: key_mask(representation, |_| true),
        }
    }

    fn is(&self, other: &Wanted) -> bool {
        let same = |a: &SelectionSet, b: &SelectionSet| std::ptr::eq(a, b) || a == b;
        self.subgraph == other.subgraph
            && same(self.needed, other.needed)
            && same(self.representation, other.representation)
    }
}

struct Found<'a> {
    wanted: Wanted<'a>,
    /// the steps with positions of the type the fetch depends on, with its waits if it has
    /// any, all that `find_in` looks at
    steps: Vec<NodeIndex>,
    /// the same without the waits, when it has any
    steps_without_waits: Option<Vec<NodeIndex>>,
    reuse: Option<ReuseMatch>,
}

#[derive(Clone)]
struct ReuseMatch {
    reuse: Arc<EntityReuse>,
    /// whether a source is only there through the fetches the waits add
    needs_waits: bool,
}

#[derive(Default)]
struct Positions<'a> {
    /// every distinct path, so `find` compares ids instead of paths
    paths: Vec<FlattenNodePath>,
    /// path ids by the hash of the path. A collision falls back to a scan of `paths`.
    ids: HashMap<u64, PathId>,
    list: Vec<Position<'a>>,
    /// the steps that filled a position, each once, in the order they did
    steps: Vec<NodeIndex>,
}

/// 24 bytes, `find` goes over all of them
struct Position<'a> {
    path: PathId,
    /// the step that filled it
    step: NodeIndex,
    selections: &'a SelectionSet,
    keys: KeyMask,
}

#[cfg(target_pointer_width = "64")]
const _: () = assert!(std::mem::size_of::<Position<'static>>() == 24);

/// Index into [`Positions::paths`], so `find` compares ids instead of paths
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PathId(u32);

impl PathId {
    fn new(index: usize) -> Self {
        Self(u32::try_from(index).expect("a plan has fewer than 2^32 paths"))
    }

    fn index(self) -> usize {
        self.0 as usize
    }
}

/// One bit per response key, see [`key_mask`]. Two fields with the same key always share the
/// bit, others may too, so it only rules things out.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct KeyMask(u64);

impl KeyMask {
    /// whether it has every bit of `other`, as a position has to for every field it's asked for
    fn has_all(self, other: KeyMask) -> bool {
        self.0 & other.0 == other.0
    }

    /// whether the two share a bit, as a position has to before it can conflict with `other`
    fn shares_any(self, other: KeyMask) -> bool {
        self.0 & other.0 != 0
    }
}

/// The steps a fetch depends on, directly or through other steps
#[derive(Clone)]
struct Dependencies(Vec<bool>);

impl Dependencies {
    fn of(fetch_graph: &FetchGraph<MultiTypeFetchStep>, index: NodeIndex) -> Self {
        let mut dependencies = Self(vec![false; fetch_graph.graph.node_bound()]);
        dependencies.add_parents(fetch_graph, index);
        dependencies
    }

    fn contains(&self, step: NodeIndex) -> bool {
        self.0[step.index()]
    }

    fn contains_all(&self, other: &Dependencies) -> bool {
        other
            .0
            .iter()
            .zip(&self.0)
            .all(|(&theirs, &ours)| !theirs || ours)
    }

    /// Adds `step` and every step it depends on
    fn add(&mut self, fetch_graph: &FetchGraph<MultiTypeFetchStep>, step: NodeIndex) {
        self.0[step.index()] = true;
        self.add_parents(fetch_graph, step);
    }

    fn add_parents(&mut self, fetch_graph: &FetchGraph<MultiTypeFetchStep>, index: NodeIndex) {
        let mut stack = vec![index];
        while let Some(step) = stack.pop() {
            for edge in fetch_graph.parents_of(step) {
                let parent = edge.source();
                if !std::mem::replace(&mut self.0[parent.index()], true) {
                    stack.push(parent);
                }
            }
        }
    }
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

struct LazyStart<'p> {
    response_path: &'p MergePath,
    start: Option<Start>,
}

impl LazyStart<'_> {
    fn get(&mut self) -> &Start {
        self.start
            .get_or_insert_with(|| Start::new(FlattenNodePath::from(self.response_path)))
    }
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
        enabled: bool,
        follow_up_types: impl IntoIterator<Item = &'a str>,
    ) -> Self {
        let mut positions = HashMap::default();
        if enabled && fetch_graph.operation_kind == OperationKind::Query {
            for index in fetch_graph.step_indices() {
                if let Some((type_name, _)) = fetch_graph.get_step_data(index).ok().and_then(wanted)
                {
                    positions.entry(type_name).or_default();
                }
            }
            for type_name in follow_up_types {
                positions.entry(type_name).or_default();
            }
        }
        Self {
            fetch_graph,
            supergraph,
            positions,
            subgraphs: Vec::new(),
            found: HashMap::default(),
            earlier_fetches: Vec::new(),
        }
    }

    /// Adds the positions `step` fills. Call it once the step's wave is planned.
    pub(super) fn add_step(&mut self, index: NodeIndex, step: &'a Step) {
        if self.positions.is_empty() || has_context(step) {
            return;
        }
        let Some(subgraph) = self.supergraph.subgraph_index(&step.service_name.0) else {
            return;
        };
        if self.subgraphs.len() <= index.index() {
            self.subgraphs.resize(index.index() + 1, 0);
        }
        self.subgraphs[index.index()] = subgraph;
        // without any, a fetch never waits for one
        if let Some((type_name, needed)) = wanted(step).filter(|_| WAIT_FOR_SAME_FETCH) {
            self.earlier_fetches.push(EarlierEntityFetch {
                index,
                id: step.id,
                service: &step.service_name.0,
                type_name,
                needed,
                depends_on: Dependencies::of(self.fetch_graph, index),
            });
        }
        self.found.clear();
        // most steps hold no position, so the start is only built for the first one
        let mut start = LazyStart {
            response_path: &step.response_path,
            start: None,
        };
        let mut path = Vec::new();
        for (definition_name, selections) in step.output.iter_selections() {
            self.walk(
                &mut start,
                &mut path,
                selections,
                definition_name,
                index,
                true,
            );
        }
    }

    /// Tells the entity fetch in `node` where the fetches it depends on already put what it
    /// asks for.
    pub(super) fn attach(&mut self, node: &mut PlanNode, index: NodeIndex, step: &'a Step) {
        let PlanNode::Flatten(flatten) = node else {
            return;
        };
        let PlanNode::Fetch(fetch) = flatten.node.as_mut() else {
            return;
        };
        let Some((reuse, waits)) = self.find(index, step) else {
            return;
        };
        fetch.entity_reuse = Some(reuse);
        if !waits.is_empty() {
            fetch.depends_on.extend(waits.iter().copied());
            fetch.depends_on.sort_unstable();
            fetch.depends_on.dedup();
            // never ones it already waited for, see `earlier_fetch_waits`
            fetch.reuse_waits = waits;
        }
    }

    /// Where a follow-up fetch (see `split_nested_entities`) can copy from: any position filled
    /// so far. It runs once its host is done, but nothing makes it wait for the others, so the
    /// executor copies what's there by then and asks the subgraph for the rest.
    pub(super) fn attach_follow_up(&mut self, node: &mut PlanNode, step: &'a Step) {
        let PlanNode::Flatten(flatten) = node else {
            return;
        };
        let PlanNode::Fetch(fetch) = flatten.node.as_mut() else {
            return;
        };
        let Some((type_name, needed)) = wanted(step) else {
            return;
        };
        let Some(representation) = step.input.selections_for_definition(type_name) else {
            return;
        };
        let subgraph = self.supergraph.subgraph_index(&step.service_name.0);
        let wanted = Wanted::new(type_name, needed, representation, subgraph);
        // positions only hold steps of planned waves, so this is all of those
        let everything = Dependencies(vec![true; self.fetch_graph.graph.node_bound()]);
        if let Some(found) = self.find_cached(&everything, None, wanted) {
            fetch.entity_reuse = Some(Arc::new(EntityReuse {
                unfilled_sources: true,
                ..(*found.reuse).clone()
            }));
        }
    }

    /// Where the fetch can copy from, and the fetches it has to wait for to do it, on top of
    /// the ones it already does
    fn find(
        &mut self,
        index: NodeIndex,
        step: &'a Step,
    ) -> Option<(Arc<EntityReuse>, FetchIdList)> {
        let (type_name, needed) = wanted(step)?;
        // nothing of the type in the response yet, not even from the same fetch earlier on
        if self.positions.get(type_name)?.steps.is_empty() {
            return None;
        }
        let representation = step.input.selections_for_definition(type_name)?;
        if reads_renamed_keys(representation) {
            return None;
        }
        let subgraph = self.supergraph.subgraph_index(&step.service_name.0);
        let wanted = Wanted::new(type_name, needed, representation, subgraph);
        let depends_on = Dependencies::of(self.fetch_graph, index);
        let (expanded, waits) = self.earlier_fetch_waits(step, &wanted, &depends_on);

        let found = self.find_cached(&depends_on, expanded.as_ref(), wanted)?;
        // waiting only pays off if it adds something to copy
        if !found.needs_waits {
            return Some((found.reuse, FetchIdList::new()));
        }
        Some((found.reuse, waits))
    }

    /// The earlier fetches to the same subgraph that asked for at least what this one does, and
    /// what the fetch would depend on if it waited for them too. `None` when there's no earlier
    /// fetch it doesn't already wait for, as for most fetches.
    fn earlier_fetch_waits(
        &self,
        step: &Step,
        wanted: &Wanted,
        depends_on: &Dependencies,
    ) -> (Option<Dependencies>, FetchIdList) {
        let mut expanded: Option<Dependencies> = None;
        let mut waits = FetchIdList::new();
        for earlier in &self.earlier_fetches {
            // already waits for it, directly or through another one
            if expanded
                .as_ref()
                .unwrap_or(depends_on)
                .contains(earlier.index)
            {
                continue;
            }
            if earlier.service != step.service_name.0
                || earlier.type_name != wanted.type_name
                || !covers(earlier.needed, wanted.needed)
            {
                continue;
            }
            // it could be held up by something this one doesn't wait for, see the module docs
            if !depends_on.contains_all(&earlier.depends_on) {
                continue;
            }
            expanded
                .get_or_insert_with(|| depends_on.clone())
                .add(self.fetch_graph, earlier.index);
            waits.push(earlier.id);
        }
        (expanded, waits)
    }

    /// [`find_in`], cached for the wave
    fn find_cached(
        &mut self,
        depends_on: &Dependencies,
        expanded: Option<&Dependencies>,
        wanted: Wanted<'a>,
    ) -> Option<ReuseMatch> {
        let positions = self.positions.get(wanted.type_name)?;
        let steps_in = |dependencies: &Dependencies| -> Vec<NodeIndex> {
            positions
                .steps
                .iter()
                .copied()
                .filter(|&step| dependencies.contains(step))
                .collect()
        };
        let steps = steps_in(expanded.unwrap_or(depends_on));
        if steps.is_empty() {
            return None;
        }
        let steps_without_waits = expanded.map(|_| steps_in(depends_on));
        let key = (
            wanted.type_name,
            wanted.needed_keys,
            wanted.representation_keys,
        );
        let found = self.found.entry(key).or_default();
        // fetches waiting for different ones leave many entries apart only by their steps, so
        // those go first, they're cheap to compare
        let cached = found.iter().find(|found| {
            found.steps == steps
                && found.steps_without_waits == steps_without_waits
                && found.wanted.is(&wanted)
        });
        if let Some(cached) = cached {
            return cached.reuse.clone();
        }
        let reuse = find_in(
            self.supergraph,
            positions,
            &self.subgraphs,
            depends_on,
            expanded,
            &wanted,
        );
        found.push(Found {
            wanted,
            steps,
            steps_without_waits,
            reuse: reuse.clone(),
        });
        reuse
    }

    /// `record` is false under a fragment whose enclosing selection has more than `__typename`
    /// for the same objects. A fetch's selection normally doesn't, every field goes into the
    /// fragment of each type. If one did, a field there like `id: username` would put another
    /// value under a key of these objects, one the conflict check in `find_in` doesn't see, so
    /// they're no source.
    fn walk(
        &mut self,
        start: &mut LazyStart<'_>,
        path: &mut Vec<Segment<'a>>,
        selections: &'a SelectionSet,
        current_type: &'a str,
        step: NodeIndex,
        record: bool,
    ) {
        if record {
            if let Some(positions) = self.positions.get_mut(current_type) {
                positions.add(start.get(), path, selections, step);
            }
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
                    path.push(Segment::Field(field.selection_identifier()));
                    for _ in 0..list_depth(field_type) {
                        path.push(Segment::List);
                    }
                    let field_type = field_type.inner_type();
                    // another object, its selection is all of it
                    self.walk(start, path, &field.selections, field_type, step, true);
                }
                SelectionItem::InlineFragment(fragment) => {
                    if fragment.type_condition != current_type {
                        path.push(Segment::TypeCondition(&fragment.type_condition));
                    }
                    let condition = &fragment.type_condition;
                    let record = record && only_typename_besides_fragments(selections);
                    self.walk(start, path, &fragment.selections, condition, step, record);
                }
                _ => {}
            }
            path.truncate(depth);
        }
    }
}

impl<'a> Positions<'a> {
    fn add(
        &mut self,
        start: &Start,
        path: &[Segment<'_>],
        selections: &'a SelectionSet,
        step: NodeIndex,
    ) {
        if self.steps.last() != Some(&step) {
            self.steps.push(step);
        }
        let path = self.intern(start, path);
        self.list.push(Position {
            path,
            step,
            selections,
            keys: key_mask(selections, |_| true),
        });
    }

    /// The id of the path `start` + `path`, it only allocates for a path it hasn't seen
    fn intern(&mut self, start: &Start, path: &[Segment<'_>]) -> PathId {
        let mut hasher = start.hasher.clone();
        for segment in path {
            segment.hash(&mut hasher);
        }
        let hash = hasher.finish();
        let is_path = |known: &FlattenNodePath| {
            known
                .as_slice()
                .strip_prefix(start.path.as_slice())
                .is_some_and(|tail| {
                    tail.len() == path.len()
                        && path
                            .iter()
                            .zip(tail)
                            .all(|(segment, known)| segment.matches(known))
                })
        };
        if let Some(&id) = self.ids.get(&hash) {
            if is_path(&self.paths[id.index()]) {
                return id;
            }
            // another path with the same hash
            if let Some(index) = self.paths.iter().position(is_path) {
                return PathId::new(index);
            }
        }
        let full: Vec<PathSegment> = start
            .path
            .as_slice()
            .iter()
            .cloned()
            .chain(path.iter().map(|segment| segment.to_path_segment()))
            .collect();
        let id = PathId::new(self.paths.len());
        self.paths.push(full.into());
        self.ids.entry(hash).or_insert(id);
        id
    }
}

/// Whether a fetch can copy from a path: not at all, only after an extra wait, or with what it
/// already waits for. Ordered, so a position can only raise it.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum SourceMatch {
    Unmatched,
    AfterExtraWait,
    Ready,
}

/// The paths the fetch can copy from, among the positions filled by steps it depends on, with
/// the waits for earlier fetches when there are any (`expanded`). And whether any of those paths
/// is only there through the waits.
fn find_in(
    supergraph: &SupergraphState,
    positions: &Positions<'_>,
    subgraphs: &[u16],
    depends_on: &Dependencies,
    expanded: Option<&Dependencies>,
    wanted: &Wanted,
) -> Option<ReuseMatch> {
    let waited = expanded.unwrap_or(depends_on);
    let mut source = vec![SourceMatch::Unmatched; positions.paths.len()];
    let mut sources = Vec::new();
    // depends only on the subgraph, and a plan has a few of them
    let mut resolves_all: HashMap<u16, bool> = HashMap::default();
    for position in &positions.list {
        let (path, step) = (position.path.index(), position.step);
        if !waited.contains(step) {
            continue;
        }
        let found = if depends_on.contains(step) {
            SourceMatch::Ready
        } else {
            SourceMatch::AfterExtraWait
        };
        // nothing this position could add
        if source[path] >= found {
            continue;
        }
        // a plan can have thousands of positions, the mask rules most of them out cheaply
        if !position.keys.has_all(wanted.needed_keys) {
            continue;
        }
        let subgraph = subgraphs[step.index()];
        // the subgraph the fetch goes to sends what it put there before: a field it only has
        // through `@provides` is under the same parent field in both, so it's provided both times
        let resolves = *resolves_all.entry(subgraph).or_insert_with(|| {
            wanted.subgraph == Some(subgraph)
                || resolves_every_field(supergraph, wanted.type_name, wanted.needed, subgraph)
        });
        if !resolves || !covers(position.selections, wanted.needed) {
            continue;
        }
        if source[path] == SourceMatch::Unmatched {
            sources.push(position.path);
        }
        source[path] = found;
    }
    if sources.is_empty() {
        return None;
    }

    // the executor matches entities by their representation, read from these positions. a user
    // alias like `id: username` would match the wrong entity there, so a position that puts
    // another field under a key of the representation rules out its path.
    for position in &positions.list {
        let path = position.path.index();
        if source[path] != SourceMatch::Unmatched
            && position.keys.shares_any(wanted.representation_keys)
            && response_keys_conflict(position.selections, wanted.representation)
        {
            source[path] = SourceMatch::Unmatched;
        }
    }
    sources.retain(|path| source[path.index()] != SourceMatch::Unmatched);
    if sources.is_empty() {
        return None;
    }
    let needs_waits = sources
        .iter()
        .any(|path| source[path.index()] == SourceMatch::AfterExtraWait);

    let copy = SelectionSet {
        items: vec![SelectionItem::InlineFragment(InlineFragmentSelection {
            type_condition: wanted.type_name.to_string(),
            selections: wanted.needed.clone(),
            skip_if: None,
            include_if: None,
        })],
    };

    let reuse = EntityReuse {
        sources: sources
            .into_iter()
            .map(|path| positions.paths[path.index()].clone())
            .collect(),
        copy: RequiresSelectionSet::from(&copy),
        unfilled_sources: false,
    };
    Some(ReuseMatch {
        reuse: Arc::new(reuse),
        needs_waits,
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
fn key_mask(selections: &SelectionSet, keep: impl Fn(&FieldSelection) -> bool + Copy) -> KeyMask {
    KeyMask(selections.items.iter().fold(0, |mask, item| match item {
        SelectionItem::Field(field) if keep(field) => {
            let key = field.selection_identifier();
            let hash = key.bytes().fold(0u32, |hash, byte| {
                hash.wrapping_mul(31).wrapping_add(byte as u32)
            });
            mask | 1 << (hash % 64)
        }
        // their fields write the same object, see `response_keys_conflict`
        SelectionItem::InlineFragment(fragment) => mask | key_mask(&fragment.selections, keep).0,
        _ => mask,
    }))
}

/// Does `source` ask for every field in `needed`, with the same response key and arguments?
/// `__typename` always counts: the executor checks it's really there before copying.
pub(crate) fn covers(source: &SelectionSet, needed: &SelectionSet) -> bool {
    needed.items.iter().all(|item| {
        let SelectionItem::Field(needed_field) = item else {
            return false;
        };
        if needed_field.name == "__typename" {
            return true;
        }
        source.items.iter().any(|item| {
            let SelectionItem::Field(field) = item else {
                return false;
            };
            field.selection_identifier() == needed_field.selection_identifier()
                && field.name == needed_field.name
                && field.arguments == needed_field.arguments
                && covers(&field.selections, &needed_field.selections)
        })
    })
}

/// Does `subgraph` resolve every field of `type_name` in `needed` itself, all the way down? Only
/// then is what it put in the response what the fetch would get.
pub(crate) fn resolves_every_field(
    supergraph: &SupergraphState,
    type_name: &str,
    needed: &SelectionSet,
    subgraph: u16,
) -> bool {
    needed.items.iter().all(|item| match item {
        SelectionItem::Field(field) if field.name == "__typename" => true,
        SelectionItem::Field(field) => resolved_by(supergraph, type_name, &field.name, subgraph)
            .is_some_and(|definition| {
                resolves_every_field(
                    supergraph,
                    definition.field_type.inner_type(),
                    &field.selections,
                    subgraph,
                )
            }),
        _ => false,
    })
}

/// The field, if `subgraph` resolves it itself. Where the field is `@external` in `subgraph`,
/// it only has it through `@provides`: another subgraph's data, as `subgraph` last saw it.
///
/// `resolvable_in` leaves out the subgraphs of a labelled `@override` too, since which of them
/// answers depends on the request. Within one plan it's always the same one, so a value one of
/// them put in the response is what a fetch to it would give again.
pub(crate) fn resolved_by<'s>(
    supergraph: &'s SupergraphState,
    type_name: &str,
    field_name: &str,
    subgraph: u16,
) -> Option<&'s SupergraphField> {
    let field = supergraph
        .definitions
        .get(type_name)?
        .fields()
        .get(field_name)?;
    let labelled_in_subgraph = || {
        field.join_field.iter().any(|join_field| {
            join_field.override_label.is_some()
                && !join_field.external
                && join_field
                    .graph_id
                    .as_deref()
                    .and_then(|graph_id| supergraph.known_subgraphs.get(graph_id))
                    .and_then(|name| supergraph.subgraph_index(name))
                    == Some(subgraph)
        })
    };
    (field.resolvable_in.binary_search(&subgraph).is_ok() || labelled_in_subgraph())
        .then_some(field)
}

/// Whether the selection above a type fragment has only `__typename` and fragments, the rest of
/// its fields in those fragments. See `walk`.
fn only_typename_besides_fragments(selections: &SelectionSet) -> bool {
    selections.items.iter().all(|item| match item {
        SelectionItem::Field(field) => field.name == "__typename" && field.alias.is_none(),
        _ => true,
    })
}

/// Whether a representation reads a value from under another key than its field's name,
/// `price: _internal_qp_alias_0` when the planner moved a field there. The conflict and key
/// checks compare response keys of what fetches wrote, so a fetch reading one like that doesn't
/// copy. It's rare.
fn reads_renamed_keys(representation: &SelectionSet) -> bool {
    fields_through_fragments(representation)
        .into_iter()
        .any(|field| field.alias.is_some() || reads_renamed_keys(&field.selections))
}

/// Fields of a fetch with `@fromContext` arguments depend on values from outside the entity,
/// so the same entity can come back different. We don't copy into or out of those.
pub(crate) fn has_context(step: &Step) -> bool {
    !step.context_variable_definitions.is_empty() || step.context_rewrites.is_some()
}

/// Only plain fields, all the way down. Fragments and `@skip`/`@include` would need the
/// copy to know about types and variables, which we don't do (yet).
pub(crate) fn is_plain(selections: &SelectionSet) -> bool {
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
    fn copies_only_what_the_source_subgraph_resolves_itself() {
        use crate::query_planner::tests::testkit::read_supergraph_state;
        let resolves = |fixture: &str, type_name: &str, field: &str, subgraph: &str| {
            let supergraph = read_supergraph_state(fixture);
            let subgraph = supergraph.subgraph_index(subgraph).expect("a subgraph");
            resolved_by(&supergraph, type_name, field, subgraph).is_some()
        };
        // `social` only has `User.name` through `@provides`, `names` owns it
        let provides = "fixture/tests/provides-copies.supergraph.graphql";
        assert!(!resolves(provides, "User", "name", "social"));
        assert!(resolves(provides, "User", "name", "names"));
        // both sides of a labelled `@override` count, the plan picks one of them
        let overrides = "fixture/tests/simple-progressive-overrides.supergraph.graphql";
        assert!(resolves(overrides, "Post", "createdAt", "a"));
        assert!(resolves(overrides, "Post", "createdAt", "b"));
    }

    fn selections(query: &str) -> SelectionSet {
        let definition = crate::query_planner::utils::parsing::parse_operation(query)
            .definitions
            .remove(0);
        let graphql_tools::parser::query::Definition::Operation(
            graphql_tools::parser::query::OperationDefinition::SelectionSet(selection),
        ) = definition
        else {
            panic!("selection");
        };
        SelectionSet::from(selection)
    }

    #[test]
    fn an_alias_inside_a_fragment_conflicts_with_the_representation() {
        let representation = selections("{ __typename id }");
        let source = selections("{ name ... on Node { id: username } }");
        let representation_keys = key_mask(&representation, |_| true);
        assert!(key_mask(&source, |_| true).shares_any(representation_keys));
        assert!(response_keys_conflict(&source, &representation));
        assert!(!response_keys_conflict(
            &selections("{ name ... on Node { id } }"),
            &representation
        ));
    }

    #[test]
    fn objects_with_fields_outside_their_fragment_are_left_out() {
        assert!(only_typename_besides_fragments(&selections(
            "{ __typename ... on User { id } }"
        )));
        assert!(!only_typename_besides_fragments(&selections(
            "{ id: username ... on User { name } }"
        )));
        assert!(!only_typename_besides_fragments(&selections(
            "{ id: __typename ... on User { name } }"
        )));
    }

    #[test]
    fn a_representation_reading_a_renamed_key_is_noticed() {
        assert!(!reads_renamed_keys(&selections("{ __typename id price }")));
        assert!(reads_renamed_keys(&selections(
            "{ __typename id price: _internal_qp_alias_0 }"
        )));
        assert!(reads_renamed_keys(&selections(
            "{ __typename id dimensions { size: _internal_qp_alias_1 } }"
        )));
    }

    #[test]
    fn a_path_gets_one_id_wherever_its_step_starts() {
        let selections = SelectionSet::default();
        let mut positions = Positions::default();
        let top = || PathSegment::Field("top".into());
        let start = |path: Vec<PathSegment>| Start::new(FlattenNodePath::from(path));

        positions.add(
            &start(vec![top()]),
            &[Segment::List],
            &selections,
            NodeIndex::new(0),
        );
        positions.add(
            &start(vec![top(), PathSegment::List]),
            &[],
            &selections,
            NodeIndex::new(0),
        );
        positions.add(
            &start(vec![]),
            &[Segment::Field("top"), Segment::List],
            &selections,
            NodeIndex::new(0),
        );
        positions.add(&start(vec![top()]), &[], &selections, NodeIndex::new(0));

        let ids: Vec<usize> = positions
            .list
            .iter()
            .map(|position| position.path.index())
            .collect();
        assert_eq!(ids, [0, 0, 0, 1]);
        assert_eq!(positions.paths.len(), 2);
    }

    #[test]
    fn a_hash_collision_still_finds_the_right_path() {
        let selections = SelectionSet::default();
        let mut positions = Positions::default();
        let start = Start::new(FlattenNodePath::from(Vec::<PathSegment>::new()));
        positions.add(
            &start,
            &[Segment::Field("first")],
            &selections,
            NodeIndex::new(0),
        );

        // point the hash of `second` at `first`
        let mut hasher = start.hasher.clone();
        Segment::Field("second").hash(&mut hasher);
        positions.ids.insert(hasher.finish(), PathId::new(0));

        positions.add(
            &start,
            &[Segment::Field("second")],
            &selections,
            NodeIndex::new(0),
        );
        positions.add(
            &start,
            &[Segment::Field("second")],
            &selections,
            NodeIndex::new(0),
        );
        let ids: Vec<usize> = positions.list.iter().map(|p| p.path.index()).collect();
        assert_eq!(ids, [0, 1, 1]);
        assert_eq!(positions.paths.len(), 2);
    }
}
