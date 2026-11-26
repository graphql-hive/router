//! `@defer` delivery.
//!
//! Each `@defer` becomes a group with its own projection plan, built once per operation and
//! cached with the normalized operation. The primary plan leaves deferred fields out.
//! At runtime a group gets one "instance" per object it's deferred on (one per list item),
//! each with its own `pending` id. An instance goes out as soon as its own fetches are done,
//! a slow sibling doesn't hold it. The instances nested in it are announced in the same part.
//!
//! Two wire formats:
//! - `incrementalSpec=v0.2`: `pending` / `incremental` / `completed` / `hasNext`
//! - `deferSpec=20220824` (older Apollo clients): `incremental: [{ data, path, label }]` / `hasNext`

use std::collections::HashMap;
use std::io::Write;
use std::ops::Range;
use std::sync::Arc;

use bytes::BufMut;
use sonic_rs::{JsonContainerTrait, JsonValueTrait};

use crate::executor::execution::plan::ExecutionResultExtensions;
use crate::executor::introspection::schema::SchemaMetadata;
use crate::executor::json_writer::write_and_escape_string;
use crate::executor::operation_filter::PathSegment;
use crate::executor::projection::error::ProjectionError;
use crate::executor::projection::plan::{FieldRecord, ProjectionPlan};
use crate::executor::projection::response::{
    project_data, trim_error_paths, write_errors, write_response_tail, Projected,
};
use crate::executor::response::graphql_error::{
    GraphQLError, GraphQLErrorPath, GraphQLErrorPathSegment,
};
use crate::executor::response::value::Value;
use crate::executor::utils::consts::TYPENAME_FIELD_NAME;
use crate::pipeline::trie::{PathIndex, Trie};
use crate::query_planner::ast::defer::{variable_bool, DeferId, DeferPart, PartPathSegment};
use crate::query_planner::ast::operation::OperationDefinition;
use crate::query_planner::ast::selection_item::SelectionItem;
use crate::query_planner::ast::selection_set::{
    DeferMarker, FieldSelection, InlineFragmentSelection, SelectionSet,
};

type Variables = Option<HashMap<String, sonic_rs::Value>>;

/// Which incremental delivery format the client asked for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum IncrementalFormat {
    /// `multipart/mixed;incrementalSpec=v0.2`, also used for SSE and WebSocket.
    #[default]
    Spec,
    /// `multipart/mixed;deferSpec=20220824`
    Legacy,
}

/// `@skip(if: $a) @include(if: $b)` on a selection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conditions {
    skip_if: Option<String>,
    include_if: Option<String>,
}

impl Conditions {
    fn new(skip_if: &Option<String>, include_if: &Option<String>) -> Self {
        Self {
            skip_if: skip_if.clone(),
            include_if: include_if.clone(),
        }
    }

    /// No `@skip` and no `@include`.
    fn is_always(&self) -> bool {
        self.skip_if.is_none() && self.include_if.is_none()
    }

    fn pass(&self, variables: &Variables) -> bool {
        let is_true = |var: &String| variable_bool(variables, var) == Some(true);
        !self.skip_if.as_ref().is_some_and(is_true) && self.include_if.as_ref().is_none_or(is_true)
    }
}

/// One step from an object towards a deferred fragment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PathSeg {
    Field {
        key: String,
        output_type: String,
        conditions: Conditions,
    },
    /// An inline fragment on the way, the object has to match it.
    Type(FragmentCondition),
}

/// What an object has to match for a fragment to apply: its type condition and its
/// `@skip`/`@include`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FragmentCondition {
    type_condition: String,
    conditions: Conditions,
}

impl FragmentCondition {
    fn new(fragment: &InlineFragmentSelection) -> Self {
        Self {
            type_condition: fragment.type_condition.clone(),
            conditions: Conditions::new(&fragment.skip_if, &fragment.include_if),
        }
    }
}

#[derive(Debug, Clone)]
pub struct DeferGroup {
    /// The group this one is nested in, `None` for the primary part.
    pub parent: Option<usize>,
    /// From the parent group's object (or the root) to the object the fragment is on.
    pub path: Vec<PathSeg>,
    pub marker: DeferMarker,
    /// The type of the object the fragment is on, the root of `plan`.
    pub type_name: String,
    pub plan: ProjectionPlan,
    /// One per fragment in the group. Normalization can split one `@defer`
    /// into several fragments (one per possible type, or per nested condition).
    fragments: Vec<FragmentCondition>,
    /// The fields under the fragment's object that another part delivers too.
    shared: Shared,
    /// How many response keys the fragment's object is from the root.
    depth: usize,
    /// Whether another group shares a field with this one, so how its fragments end is kept.
    watched: bool,
}

/// Response keys that more than one part delivers, by key, nested the way the fields are. A
/// null a deferred fragment's projection puts on one of these would wipe data another part
/// sends, whichever goes out first, so the fragment fails then, see `nulls_shared`.
#[derive(Debug, Clone, Default)]
struct Shared(HashMap<String, SharedField>);

#[derive(Debug, Clone)]
struct SharedField {
    /// How each other part has the field. One of them has to hold for the field to be shared
    /// in a request. Empty when a part has it whatever the variables and the type are.
    when: Vec<Selected>,
    /// The type the field returns, for an object that comes without a `__typename`. Empty when
    /// the schema doesn't have the field.
    output_type: String,
    fields: Shared,
}

/// A part that has a field, and what it has it under.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Selected {
    /// The group, `None` for the primary part.
    owner: Option<usize>,
    /// The `@skip`/`@include`s, from the root.
    conditions: Vec<Conditions>,
    /// The type conditions from the root, each with how many response keys from the root the
    /// object it's on is.
    types: Vec<(usize, String)>,
}

impl Shared {
    /// Marks the groups that have one of these fields.
    fn owners(&self, groups: &mut [bool]) {
        for field in self.0.values() {
            for owner in field.when.iter().filter_map(|selected| selected.owner) {
                groups[owner] = true;
            }
            field.fields.owners(groups);
        }
    }
}

/// Who selects a field, by its response keys from the root: the groups, `None` for the primary
/// part, each with what it has the field under. Only there while the plans are built.
#[derive(Default)]
struct Position {
    owners: Vec<Selected>,
    output_type: String,
    children: HashMap<String, Position>,
}

impl Position {
    fn add(&mut self, path: &[String], selected: Selected, output_type: &str) {
        let mut node = self;
        for key in path {
            node = node.children.entry(key.clone()).or_default();
        }
        node.output_type = output_type.to_string();
        if !node.owners.contains(&selected) {
            node.owners.push(selected);
        }
    }

    /// The fields under this one that an owner `outside` picks selects.
    fn shared(&self, outside: &impl Fn(Option<usize>) -> bool) -> Shared {
        let mut shared = HashMap::new();
        for (key, child) in &self.children {
            let mut when: Vec<&Selected> = child
                .owners
                .iter()
                .filter(|selected| outside(selected.owner))
                .collect();
            if when.is_empty() {
                continue;
            }
            // the primary part has it whatever the request is, the others don't matter then
            if when.iter().any(|selected| {
                selected.owner.is_none()
                    && selected.conditions.is_empty()
                    && selected.types.is_empty()
            }) {
                when.clear();
            }
            shared.insert(
                key.clone(),
                SharedField {
                    when: when.into_iter().cloned().collect(),
                    output_type: child.output_type.clone(),
                    fields: child.shared(outside),
                },
            );
        }
        Shared(shared)
    }
}

#[derive(Debug)]
pub struct DeferPlans {
    pub groups: Vec<DeferGroup>,
}

impl DeferPlans {
    /// `None` when the operation has no `@defer`.
    pub fn build(operation: &OperationDefinition, schema: &SchemaMetadata) -> Option<Self> {
        let root = schema.expect_root_type_name(operation.operation_kind.as_ref());
        let mut builder = Builder {
            schema,
            groups: Vec::new(),
            fragments: Vec::new(),
            positions: Position::default(),
            keys: Vec::new(),
            conditions: Vec::new(),
            types: Vec::new(),
            anchors: Vec::new(),
        };
        builder.walk(&operation.selection_set, root, &mut Vec::new(), None, &[]);
        if builder.groups.is_empty() {
            return None;
        }
        for index in 0..builder.groups.len() {
            let position = builder.anchors[index]
                .iter()
                .try_fold(&builder.positions, |node, key| node.children.get(key));
            // the group itself and the ones nested in it go out with it or after it, every
            // other part is another one
            let outside = |owner: Option<usize>| {
                !std::iter::successors(owner, |owner| builder.groups[*owner].parent)
                    .any(|owner| owner == index)
            };
            if let Some(position) = position {
                builder.groups[index].shared = position.shared(&outside);
            }
            builder.groups[index].depth = builder.anchors[index].len();
        }
        let mut watched = vec![false; builder.groups.len()];
        for group in &builder.groups {
            group.shared.owners(&mut watched);
        }
        for (group, watched) in builder.groups.iter_mut().zip(watched) {
            group.watched = watched;
        }
        for (group, fragments) in builder.groups.iter_mut().zip(builder.fragments) {
            let selection_set = SelectionSet {
                items: fragments
                    .into_iter()
                    .map(SelectionItem::InlineFragment)
                    .collect(),
            };
            group.plan =
                ProjectionPlan::from_selection_set(&selection_set, schema, &group.type_name);
        }
        Some(Self {
            groups: builder.groups,
        })
    }

    /// Nulls the fields authorization rejected, the same as the primary plan gets.
    /// `trie` has the rejected paths from the root, through fields and fragments,
    /// deferred fragments included.
    pub(crate) fn rewrite(self: &Arc<Self>, trie: &Trie) -> Arc<Self> {
        let mut rewritten: Option<Vec<DeferGroup>> = None;
        for (index, group) in self.groups.iter().enumerate() {
            let Some(anchor) = self.trie_position(index, trie) else {
                continue;
            };
            // fields of the group's fragments sit under `Fragment(type)` in the trie. A field
            // with a type guard finds its fragment from the anchor, one without starts inside it
            let mut starts = vec![anchor];
            for fragment in &group.fragments {
                if let Some((inside, _)) = trie.find_segment_at_position(
                    anchor,
                    PathSegment::Fragment(&fragment.type_condition),
                ) {
                    if !starts.contains(&inside) {
                        starts.push(inside);
                    }
                }
            }
            if let Some(plan) = group.plan.rewrite_from(trie, &starts) {
                rewritten.get_or_insert_with(|| self.groups.clone())[index].plan = plan;
            }
        }
        match rewritten {
            Some(groups) => Arc::new(Self { groups }),
            None => Arc::clone(self),
        }
    }

    /// Where the object a group is on sits in `trie`, `None` when nothing under it is marked.
    fn trie_position(&self, index: usize, trie: &Trie) -> Option<PathIndex> {
        let mut chain = vec![index];
        while let Some(parent) = self.groups[*chain.last()?].parent {
            chain.push(parent);
        }
        let mut position = PathIndex::root();
        for group in chain.into_iter().rev() {
            for segment in &self.groups[group].path {
                let segment = match segment {
                    PathSeg::Field { key, .. } => PathSegment::Field(key),
                    PathSeg::Type(fragment) => PathSegment::Fragment(&fragment.type_condition),
                };
                position = trie.find_segment_at_position(position, segment)?.0;
            }
        }
        Some(position)
    }

    /// Whether anything will be deferred with these variables: an active `@defer` that
    /// `@skip`/`@include` don't leave out. When nothing is, the response is a regular single
    /// response.
    pub fn has_active(&self, variables: &Variables) -> bool {
        (0..self.groups.len()).any(|index| {
            self.groups[index].marker.is_active(variables) && self.included(index, variables)
        })
    }

    /// Whether `@skip`/`@include` keep a group in: everything on the way to it, at least one of
    /// its fragments, and the same for every group it's nested in.
    fn included(&self, mut index: usize, variables: &Variables) -> bool {
        loop {
            let group = &self.groups[index];
            let on_the_way = group.path.iter().all(|segment| match segment {
                PathSeg::Field { conditions, .. } => conditions.pass(variables),
                PathSeg::Type(fragment) => fragment.conditions.pass(variables),
            });
            let any_fragment = group
                .fragments
                .iter()
                .any(|fragment| fragment.conditions.pass(variables));
            if !on_the_way || !any_fragment {
                return false;
            }
            match group.parent {
                Some(parent) => index = parent,
                None => return true,
            }
        }
    }

    /// The closest active ancestor, and the path from it, for an active group. A group in an
    /// inactive one is still active, it's just delivered relative to that ancestor.
    fn delivery(&self, index: usize, variables: &Variables) -> Delivery {
        let group = &self.groups[index];
        let mut path = group.path.clone();
        let mut parent = group.parent;
        while let Some(p) = parent {
            if self.groups[p].marker.is_active(variables) {
                break;
            }
            let mut joined = self.groups[p].path.clone();
            joined.append(&mut path);
            path = joined;
            parent = self.groups[p].parent;
        }
        Delivery { parent, path }
    }
}

/// Where an active group is delivered: under its closest active ancestor (`None` for the
/// primary part), at `path` from that ancestor's object.
struct Delivery {
    parent: Option<usize>,
    path: Vec<PathSeg>,
}

struct Builder<'s> {
    schema: &'s SchemaMetadata,
    groups: Vec<DeferGroup>,
    fragments: Vec<Vec<InlineFragmentSelection>>,
    /// Who selects what, to tell which fields more than one part delivers.
    positions: Position,
    /// The response keys from the root to where the walk is.
    keys: Vec<String>,
    /// The `@skip`/`@include`s from the root to where the walk is.
    conditions: Vec<Conditions>,
    /// The type conditions from the root to where the walk is, see `Selected::types`.
    types: Vec<(usize, String)>,
    /// The response keys from the root to each group's object.
    anchors: Vec<Vec<String>>,
}

impl Builder<'_> {
    /// `sent` are the selection sets of the enclosing parts on the same object as `set`, the
    /// primary part's and the deferred fragments' around it. Their fields go out before the
    /// fragments in `set`, so those leave them out.
    fn walk<'s>(
        &mut self,
        set: &'s SelectionSet,
        parent_type: &str,
        path: &mut Vec<PathSeg>,
        group: Option<usize>,
        sent: &[&'s SelectionSet],
    ) {
        for item in &set.items {
            match item {
                SelectionItem::Field(field) => {
                    let conditions = Conditions::new(&field.skip_if, &field.include_if);
                    let depth = self.conditions.len();
                    if !conditions.is_always() {
                        self.conditions.push(conditions.clone());
                    }
                    self.keys.push(field.selection_identifier().to_string());
                    let info = self
                        .schema
                        .get_type_fields(parent_type)
                        .and_then(|fields| fields.get(&field.name));
                    self.positions.add(
                        &self.keys,
                        Selected {
                            owner: group,
                            conditions: self.conditions.clone(),
                            types: self.types.clone(),
                        },
                        info.map_or("", |info| &info.output_type_name),
                    );
                    let info = info.filter(|_| !field.selections.is_empty());
                    if let Some(info) = info {
                        let output_type = info.output_type_name.clone();
                        path.push(PathSeg::Field {
                            key: field.selection_identifier().to_string(),
                            output_type: output_type.clone(),
                            conditions,
                        });
                        let sent = sent_fields(sent, field.selection_identifier())
                            .map(|sent| &sent.selections)
                            .collect::<Vec<_>>();
                        self.walk(&field.selections, &output_type, path, group, &sent);
                        path.pop();
                    }
                    self.keys.pop();
                    self.conditions.truncate(depth);
                }
                SelectionItem::InlineFragment(fragment) => {
                    let conditions = Conditions::new(&fragment.skip_if, &fragment.include_if);
                    let depth = self.conditions.len();
                    if !conditions.is_always() {
                        self.conditions.push(conditions);
                    }
                    let types = self.types.len();
                    if fragment.type_condition != parent_type {
                        self.types
                            .push((self.keys.len(), fragment.type_condition.clone()));
                    }
                    let Some(marker) = &fragment.defer else {
                        path.push(PathSeg::Type(FragmentCondition::new(fragment)));
                        self.walk(
                            &fragment.selections,
                            &fragment.type_condition,
                            path,
                            group,
                            sent,
                        );
                        path.pop();
                        self.conditions.truncate(depth);
                        self.types.truncate(types);
                        continue;
                    };
                    let index = self.group_for(group, path, marker, parent_type);
                    self.groups[index]
                        .fragments
                        .push(FragmentCondition::new(fragment));
                    // the part around this fragment goes out first, with `set`'s fields
                    let mut sent_here = sent.to_vec();
                    sent_here.push(set);
                    let mut own = fragment.clone();
                    own.defer = None;
                    own.selections = without_sent(&fragment.selections, &sent_here);
                    self.fragments[index].push(own);
                    // nested defers are relative to this group's object
                    let mut inner = vec![PathSeg::Type(FragmentCondition::new(fragment))];
                    self.walk(
                        &fragment.selections,
                        &fragment.type_condition,
                        &mut inner,
                        Some(index),
                        &sent_here,
                    );
                    self.conditions.truncate(depth);
                    self.types.truncate(types);
                }
                SelectionItem::FragmentSpread(_) => {}
            }
        }
    }

    /// Fragments with the same parent group, path and `@defer` are one group.
    /// That puts back together what normalization split up.
    fn group_for(
        &mut self,
        parent: Option<usize>,
        path: &[PathSeg],
        marker: &DeferMarker,
        type_name: &str,
    ) -> usize {
        if let Some(index) = self
            .groups
            .iter()
            .position(|g| g.parent == parent && g.path == path && g.marker == *marker)
        {
            return index;
        }
        self.groups.push(DeferGroup {
            parent,
            path: path.to_vec(),
            marker: marker.clone(),
            type_name: type_name.to_string(),
            plan: ProjectionPlan::default(),
            fragments: Vec::new(),
            shared: Shared::default(),
            depth: 0,
            watched: false,
        });
        self.fragments.push(Vec::new());
        self.anchors.push(self.keys.clone());
        self.groups.len() - 1
    }
}

/// The fields of `sent` with this response key that go out whatever the variables are. A field
/// under `@skip`/`@include`, or in a fragment, may not have gone out, so it doesn't count.
fn sent_fields<'a, 's: 'a>(
    sent: &'a [&'s SelectionSet],
    key: &'a str,
) -> impl Iterator<Item = &'s FieldSelection> + 'a {
    sent.iter()
        .flat_map(|set| set.items.iter())
        .filter_map(move |item| match item {
            SelectionItem::Field(field)
                if field.selection_identifier() == key
                    && field.skip_if.is_none()
                    && field.include_if.is_none() =>
            {
                Some(field)
            }
            _ => None,
        })
}

/// `set` without the fields `sent` delivers already, so a deferred fragment doesn't send them
/// again. A field with a selection keeps what's new in it. Nested deferred fragments stay.
// ponytail: fields `sent` has in fragments or under conditions are sent again, it's only
// bandwidth
fn without_sent(set: &SelectionSet, sent: &[&SelectionSet]) -> SelectionSet {
    if sent.is_empty() {
        return set.clone();
    }
    let mut out = SelectionSet::default();
    for item in &set.items {
        match item {
            SelectionItem::Field(field) => {
                let sent_children = sent_fields(sent, field.selection_identifier())
                    .map(|sent| &sent.selections)
                    .collect::<Vec<_>>();
                if sent_children.is_empty() {
                    out.items.push(item.clone());
                    continue;
                }
                if field.selections.is_empty() {
                    continue;
                }
                let selections = without_sent(&field.selections, &sent_children);
                if !selections.is_empty() {
                    let mut field = field.clone();
                    field.selections = selections;
                    out.items.push(SelectionItem::Field(field));
                }
            }
            SelectionItem::InlineFragment(fragment) if fragment.defer.is_none() => {
                let selections = without_sent(&fragment.selections, sent);
                if !selections.is_empty() {
                    let mut fragment = fragment.clone();
                    fragment.selections = selections;
                    out.items.push(SelectionItem::InlineFragment(fragment));
                }
            }
            item => out.items.push(item.clone()),
        }
    }
    out
}

/// An object a deferred fragment hangs off, found while walking the data.
struct Anchor {
    path: Vec<GraphQLErrorPathSegment>,
    type_name: String,
}

/// Follows `path` in projected output. The anchor is alive when there's an object there.
/// Null propagation in the output can kill an anchor that's fine in the raw data.
fn alive_in_output(output: &sonic_rs::Value, path: &[GraphQLErrorPathSegment]) -> bool {
    let mut current = output;
    for segment in path {
        let next = match segment {
            GraphQLErrorPathSegment::String(key) => current.get(key.as_str()),
            GraphQLErrorPathSegment::Index(index) => current.get(*index),
        };
        match next {
            Some(next) => current = next,
            None => return false,
        }
    }
    current.is_object()
}

/// How far into `segments` the plan's fields go. Index segments are free.
fn matched_len(plan: &ProjectionPlan, segments: &[GraphQLErrorPathSegment]) -> usize {
    let mut fields: Vec<&FieldRecord> = plan.root_fields().iter().collect();
    let mut matched = 0;
    for (position, segment) in segments.iter().enumerate() {
        if let GraphQLErrorPathSegment::String(key) = segment {
            if !fields.iter().any(|field| plan.response_key(field) == key) {
                break;
            }
            fields = fields
                .iter()
                .filter(|field| plan.response_key(field) == key)
                .flat_map(|field| plan.fields(field.children))
                .collect();
        }
        matched = position + 1;
    }
    matched
}

/// What `project_data` writes first.
const DATA_PREFIX: &[u8] = b"{\"data\":";

const GENERIC_NULL_ERROR: &str = "Cannot return null for non-nullable field.";

/// A deferred fragment on one object: announced in a `pending` entry, not delivered yet.
struct Pending {
    id: usize,
    group: usize,
    /// From the root of the response.
    path: Vec<GraphQLErrorPathSegment>,
    type_name: String,
    /// Errors that belong to it, held until it goes out.
    errors: Vec<GraphQLError>,
}

/// Where a held error goes.
enum Owner {
    /// The pending fragment at this index, with how much of the error's path it has. The path
    /// is cut there, at 0 it's dropped.
    Fragment(usize, usize),
    /// A fragment of this part that isn't announced yet.
    Later(DeferId),
}

/// Whether `path` is one of a part's objects: `part_path` with any list index.
pub fn is_part_path(part_path: &[PartPathSegment], path: &[GraphQLErrorPathSegment]) -> bool {
    part_path.len() == path.len()
        && part_path
            .iter()
            .zip(path)
            .all(|(expected, segment)| match (expected, segment) {
                (PartPathSegment::Key(expected), GraphQLErrorPathSegment::String(key)) => {
                    key == expected
                }
                (PartPathSegment::AnyIndex, GraphQLErrorPathSegment::Index(_)) => true,
                _ => false,
            })
}

/// Writes an incremental response part by part, as the data comes in.
/// [`Self::primary`] once the primary data is there, then [`Self::next`] whenever some
/// announced fragments have all their data, until it returns `None`.
pub struct IncrementalWriter<'p> {
    plans: &'p DeferPlans,
    primary_plan: &'p ProjectionPlan,
    schema: &'p SchemaMetadata,
    variables: &'p Variables,
    format: IncrementalFormat,
    /// Where each active group is delivered, `None` for inactive ones.
    deliveries: Vec<Option<Delivery>>,
    pending: Vec<Pending>,
    next_id: usize,
    /// Pending fragments with an id below this one were announced in a part that went out.
    /// The ones above it came up while a part was written, the client never heard of them.
    announced: usize,
    /// The plan's deferred parts, to find the fragments an error from a deferred fetch is for.
    parts: &'p [DeferPart],
    /// Errors from a deferred fetch whose fragment isn't announced yet, with the fetch's part.
    unplaced: Vec<(DeferId, GraphQLError)>,
    /// How the fragments of watched groups ended: their group, their path and whether they
    /// went out with data.
    // ponytail: a list, read only when a null lands on a shared field. A map if that's ever hot
    finished: Vec<(usize, Vec<GraphQLErrorPathSegment>, bool)>,
}

/// Where a walk down a fragment's data is, see `IncrementalWriter::nulls_shared`.
struct Place {
    /// From the root of the response.
    path: Vec<GraphQLErrorPathSegment>,
    /// The type of every object on the way, by how many response keys from the root it is.
    /// `None` when there's nothing to tell it by.
    types: Vec<Option<String>>,
}

/// How much of `path` leads to the object `depth` response keys from the root, its list
/// indexes included.
fn object_path(path: &[GraphQLErrorPathSegment], depth: usize) -> &[GraphQLErrorPathSegment] {
    let mut keys = 0;
    for (index, segment) in path.iter().enumerate() {
        if matches!(segment, GraphQLErrorPathSegment::String(_)) {
            if keys == depth {
                return &path[..index];
            }
            keys += 1;
        }
    }
    path
}

fn type_name_of(object: &[(&str, Value)]) -> Option<String> {
    Value::object_get(object, TYPENAME_FIELD_NAME)
        .and_then(Value::as_str)
        .map(str::to_string)
}

impl<'p> IncrementalWriter<'p> {
    pub fn new(
        plans: &'p DeferPlans,
        primary_plan: &'p ProjectionPlan,
        schema: &'p SchemaMetadata,
        variables: &'p Variables,
        format: IncrementalFormat,
        parts: &'p [DeferPart],
    ) -> Self {
        let deliveries = (0..plans.groups.len())
            .map(|index| {
                plans.groups[index]
                    .marker
                    .is_active(variables)
                    .then(|| plans.delivery(index, variables))
            })
            .collect();
        Self {
            plans,
            primary_plan,
            schema,
            variables,
            format,
            deliveries,
            pending: Vec::new(),
            next_id: 0,
            announced: 0,
            parts,
            unplaced: Vec::new(),
            finished: Vec::new(),
        }
    }

    /// Whether more parts are coming.
    pub fn has_next(&self) -> bool {
        !self.pending.is_empty()
    }

    fn type_matches(&self, type_name: &str, condition: &str) -> bool {
        type_name == condition
            // without a `__typename` we only know the abstract type, let it through
            || self.schema.is_interface_type(type_name)
            || self.schema.is_union_type(type_name)
            || self
                .schema
                .possible_types
                .entity_satisfies_type_condition(type_name, condition)
    }

    /// Whether `fragment` applies to an object of type `type_name`.
    fn applies(&self, fragment: &FragmentCondition, type_name: &str) -> bool {
        fragment.conditions.pass(self.variables)
            && self.type_matches(type_name, &fragment.type_condition)
    }

    /// Every object at `segs` under `value`, lists expanded.
    fn anchors<'v>(
        &self,
        value: &'v Value<'v>,
        segs: &[PathSeg],
        static_type: &str,
        concrete: &mut Vec<GraphQLErrorPathSegment>,
        out: &mut Vec<Anchor>,
    ) {
        match value {
            Value::Array(items) => {
                for (index, item) in items.iter().enumerate() {
                    concrete.push(GraphQLErrorPathSegment::Index(index));
                    self.anchors(item, segs, static_type, concrete, out);
                    concrete.pop();
                }
            }
            Value::Object(object) => {
                let type_name = Value::object_get(object, TYPENAME_FIELD_NAME)
                    .and_then(Value::as_str)
                    .unwrap_or(static_type);
                match segs.split_first() {
                    None => out.push(Anchor {
                        path: concrete.clone(),
                        type_name: type_name.to_string(),
                    }),
                    Some((PathSeg::Type(fragment), rest)) => {
                        if self.applies(fragment, type_name) {
                            self.anchors(value, rest, static_type, concrete, out);
                        }
                    }
                    Some((
                        PathSeg::Field {
                            key,
                            output_type,
                            conditions,
                        },
                        rest,
                    )) => {
                        if !conditions.pass(self.variables) {
                            return;
                        }
                        if let Some(child) = Value::object_get(object, key) {
                            concrete.push(GraphQLErrorPathSegment::String(key.clone()));
                            self.anchors(child, rest, output_type, concrete, out);
                            concrete.pop();
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn children_of(&self, parent: Option<usize>) -> Vec<usize> {
        (0..self.deliveries.len())
            .filter(|index| {
                self.deliveries[*index]
                    .as_ref()
                    .is_some_and(|delivery| delivery.parent == parent)
            })
            .collect()
    }

    /// The objects under `base` the fragments of `groups` are on: each with its group, its path
    /// from `base` and its type.
    fn anchors_of(&self, groups: &[usize], base: &Value, base_type: &str) -> Vec<(usize, Anchor)> {
        let mut found = Vec::new();
        for &group in groups {
            let Some(delivery) = &self.deliveries[group] else {
                continue;
            };
            let mut anchors = Vec::new();
            self.anchors(
                base,
                &delivery.path,
                base_type,
                &mut Vec::new(),
                &mut anchors,
            );
            found.extend(
                anchors
                    .into_iter()
                    .filter(|anchor| {
                        self.plans.groups[group]
                            .fragments
                            .iter()
                            .any(|fragment| self.applies(fragment, &anchor.type_name))
                    })
                    .map(|anchor| (group, anchor)),
            );
        }
        found
    }

    /// Announces a fragment, it goes out in a later part.
    fn push_pending(
        &mut self,
        group: usize,
        path: Vec<GraphQLErrorPathSegment>,
        type_name: String,
    ) {
        self.pending.push(Pending {
            id: self.next_id,
            group,
            path,
            type_name,
            errors: Vec::new(),
        });
        self.next_id += 1;
    }

    /// How far into `segments` the fragment's plan goes, `None` when the path isn't under it.
    fn matched(&self, item: &Pending, segments: &[GraphQLErrorPathSegment]) -> Option<usize> {
        segments.starts_with(&item.path).then(|| {
            item.path.len()
                + matched_len(
                    &self.plans.groups[item.group].plan,
                    &segments[item.path.len()..],
                )
        })
    }

    /// The pending fragment in `candidates` that has more of the error's path than `floor`.
    fn owner(
        &self,
        error: &GraphQLError,
        candidates: Range<usize>,
        floor: usize,
    ) -> Option<(usize, usize)> {
        let segments = &error.path.as_ref()?.segments;
        let mut best: Option<(usize, usize)> = None;
        for index in candidates {
            if let Some(matched) = self.matched(&self.pending[index], segments) {
                if matched > best.map_or(floor, |(_, m)| m) {
                    best = Some((index, matched));
                }
            }
        }
        best
    }

    /// The pending fragment in `candidates` an error from a fetch of `part` goes to: of the
    /// fragments for that part, the one with the most of the error's path (and how much), or
    /// the first one with none of it, its path isn't in the response. `None` when none of them
    /// is for that part.
    fn owner_in_part(
        &self,
        error: &GraphQLError,
        part: DeferId,
        candidates: Range<usize>,
    ) -> Option<(usize, usize)> {
        let part = &self.parts[part.index()];
        let segments = error.path.as_ref().map(|path| path.segments.as_slice());
        let mut first = None;
        let mut best: Option<(usize, usize)> = None;
        for index in candidates {
            let item = &self.pending[index];
            if self.plans.groups[item.group].marker.position != part.position
                || !is_part_path(&part.path, &item.path)
            {
                continue;
            }
            first.get_or_insert(index);
            if let Some(matched) = segments.and_then(|segments| self.matched(item, segments)) {
                if matched > best.map_or(0, |(_, m)| m) {
                    best = Some((index, matched));
                }
            }
        }
        best.or(first.map(|index| (index, 0)))
    }

    /// Keeps an error with its fragment until that goes out.
    fn hold(&mut self, mut error: GraphQLError, owner: Owner) {
        match owner {
            Owner::Fragment(index, matched) => {
                truncate_path(&mut error, matched);
                self.pending[index].errors.push(error);
            }
            Owner::Later(part) => self.unplaced.push((part, error)),
        }
    }

    /// Where an error from a fetch of `part`, or one with no part (`None`), goes in a later
    /// part: its fragment, or `None` for one going out now.
    fn owner_later(
        &self,
        error: &GraphQLError,
        part: Option<DeferId>,
        candidates: Range<usize>,
    ) -> Option<Owner> {
        match part {
            Some(part) => Some(
                self.owner_in_part(error, part, candidates)
                    .map_or(Owner::Later(part), |(index, matched)| {
                        Owner::Fragment(index, matched)
                    }),
            ),
            None => self
                .owner(error, candidates, 0)
                .map(|(index, matched)| Owner::Fragment(index, matched)),
        }
    }

    /// The primary part. `errors` are what came in so far, the ones on deferred fields wait for
    /// their fragment. `origins` has the part of the fetch each error came from, `None` for
    /// the primary part's fetches and anything else. `for_client` gets the errors as they go
    /// out, the primary part's (`true`) and the held ones (`false`) apart, so what it decides
    /// for the primary part (like a status code) only counts the primary part's errors.
    #[allow(clippy::too_many_arguments)]
    pub fn primary(
        &mut self,
        data: &Value,
        errors: Vec<GraphQLError>,
        origins: Vec<Option<DeferId>>,
        extensions: &ExecutionResultExtensions<'_>,
        operation_type_name: &str,
        response_size_estimate: usize,
        mut for_client: impl FnMut(Vec<GraphQLError>, bool) -> Vec<GraphQLError>,
    ) -> Result<Vec<u8>, ProjectionError> {
        let Projected {
            buffer: mut out,
            errors: projection_errors,
            nulled,
        } = project_data(
            data,
            Vec::new(),
            operation_type_name,
            self.primary_plan,
            self.variables,
            Vec::with_capacity(response_size_estimate),
            self.schema,
        )?;
        let data_end = out.len();

        let top = self.children_of(None);
        let anchors = if top.is_empty() {
            Vec::new()
        } else {
            self.anchors_of(&top, data, operation_type_name)
        };
        if !anchors.is_empty() {
            // a null that projection put in can take out an anchor that's there in the data. Only
            // then the output is read back, to tell which
            let tree = match nulled {
                true => Some(read_back(&out[DATA_PREFIX.len()..data_end])?),
                false => None,
            };
            for (group, anchor) in anchors {
                if tree
                    .as_ref()
                    .is_some_and(|tree| !alive_in_output(tree, &anchor.path))
                {
                    continue;
                }
                self.push_pending(group, anchor.path, anchor.type_name);
            }
        }

        // errors on deferred fields wait for their fragment. One from a deferred fetch is its
        // fragment's even without a path, a subgraph that's down sends none. Any other goes by
        // the path it came with. Split before `for_client`
        let mut own = Vec::new();
        let mut held = Vec::new();
        let mut held_by = Vec::new();
        let origins = origins.into_iter().chain(std::iter::repeat(None));
        for (error, origin) in errors.into_iter().zip(origins) {
            let owner = match origin {
                Some(part) => match self.owner_in_part(&error, part, 0..self.pending.len()) {
                    Some((index, matched)) => Some(Owner::Fragment(index, matched)),
                    // not announced yet, and with nothing pending it never will be
                    None if self.pending.is_empty() => None,
                    None => Some(Owner::Later(part)),
                },
                None => {
                    let floor = error
                        .path
                        .as_ref()
                        .map_or(0, |path| matched_len(self.primary_plan, &path.segments));
                    self.owner(&error, 0..self.pending.len(), floor)
                        .map(|(index, matched)| Owner::Fragment(index, matched))
                }
            };
            match owner {
                None => own.push(error),
                Some(owner) => {
                    held.push(error);
                    held_by.push(owner);
                }
            }
        }
        if !held.is_empty() {
            // one out for every one in, in the same order
            for (error, owner) in for_client(held, false).into_iter().zip(held_by) {
                self.hold(error, owner);
            }
        }
        let mut primary_errors = projection_errors;
        if !own.is_empty() {
            primary_errors.extend(for_client(own, true));
        }
        if !primary_errors.is_empty() {
            trim_error_paths(&mut primary_errors, self.primary_plan);
        }

        write_response_tail(&mut out, &primary_errors, extensions)?;
        if self.format == IncrementalFormat::Spec {
            write_pending(&mut out, &self.pending, self.plans);
        }
        write_has_next(&mut out, self.has_next());
        out.put_u8(b'}');
        self.announced = self.next_id;
        Ok(out)
    }

    /// Whether an announced fragment that `ready` says has its data is waiting. `ready` gets
    /// the fragment's group and its path from the root.
    pub fn has_ready(
        &self,
        ready: impl Fn(&DeferGroup, &[GraphQLErrorPathSegment]) -> bool,
    ) -> bool {
        self.pending
            .iter()
            .any(|item| ready(&self.plans.groups[item.group], &item.path))
    }

    /// The next part: every announced fragment that `ready` says has its data (see
    /// [`Self::has_ready`]), projected from `data`, plus `pending` entries for the fragments
    /// nested in them. The rest stays pending. `errors` came up since the last part.
    /// `None` when nothing is ready.
    pub fn next(
        &mut self,
        data: &Value,
        errors: Vec<GraphQLError>,
        origins: Vec<Option<DeferId>>,
        ready: impl Fn(&DeferGroup, &[GraphQLErrorPathSegment]) -> bool,
    ) -> Result<Option<Vec<u8>>, ProjectionError> {
        self.next_written_by(data, errors, origins, ready, Self::write_part)
    }

    /// [`Self::next`], with the part written by `write`. Tests make it fail.
    fn next_written_by(
        &mut self,
        data: &Value,
        errors: Vec<GraphQLError>,
        origins: Vec<Option<DeferId>>,
        ready: impl Fn(&DeferGroup, &[GraphQLErrorPathSegment]) -> bool,
        write: impl FnOnce(
            &Self,
            &[(Pending, Option<&[u8]>)],
            usize,
        ) -> Result<Vec<u8>, ProjectionError>,
    ) -> Result<Option<Vec<u8>>, ProjectionError> {
        let current_len = self.pending.len();
        let going: Vec<usize> = (0..current_len)
            .filter(|&index| {
                let item = &self.pending[index];
                ready(&self.plans.groups[item.group], &item.path)
            })
            .collect();
        let Some(&first_going) = going.first() else {
            return Ok(None);
        };

        // the primary part is gone already, errors with no fragment go with one going out now
        self.place(errors, origins, first_going);

        // the fragments are projected into one buffer, one after another, so they don't each grow
        // their own. `outputs` says where each one's data is.
        let mut buffer = Vec::new();
        let mut outputs = Vec::with_capacity(going.len());
        for &index in &going {
            let output = self.project_pending(index, data, &mut buffer)?;
            let item = &self.pending[index];
            if self.plans.groups[item.group].watched {
                self.finished
                    .push((item.group, item.path.clone(), output.is_some()));
            }
            outputs.push(output);
        }

        // `going` and the fragments nested in them are now announced, the rest stays
        let mut goes = vec![false; self.pending.len()];
        for &index in &going {
            goes[index] = true;
        }
        let mut goes = goes.into_iter();
        let (mut delivered, staying): (Vec<Pending>, Vec<Pending>) =
            std::mem::take(&mut self.pending)
                .into_iter()
                .partition(|_| goes.next().unwrap_or_default());
        self.pending = staying;
        // nothing is left to announce, so errors still waiting for their fragment go now
        if self.pending.is_empty() && !self.unplaced.is_empty() {
            if let Some(item) = delivered.first_mut() {
                item.errors
                    .extend(self.unplaced.drain(..).map(|(_, error)| error));
            }
        }
        let delivered: Vec<(Pending, Option<&[u8]>)> = delivered
            .into_iter()
            .zip(outputs)
            .map(|(item, output)| (item, output.map(|output| &buffer[output])))
            .collect();
        // only the fragments announced in this part, the others were announced before
        let announced = current_len - delivered.len();
        let part = write(self, &delivered, announced);
        if part.is_err() {
            // they didn't go out, the failure part still has to end them
            self.pending
                .extend(delivered.into_iter().map(|(item, _)| item));
        }
        let part = part?;
        self.announced = self.next_id;
        Ok(Some(part))
    }

    /// The last part, for when the rest can't come: the request timed out, or a part couldn't
    /// be written. Every announced fragment goes out with no data and `error`, so the client
    /// isn't left waiting for any of them. Fragments the client never heard of are left out.
    /// `errors` came up since the last part, `error` and they come as the client gets them.
    pub fn fail_pending(
        &mut self,
        errors: Vec<GraphQLError>,
        origins: Vec<Option<DeferId>>,
        error: GraphQLError,
    ) -> Result<Vec<u8>, ProjectionError> {
        if !self.pending.is_empty() {
            self.place(errors, origins, 0);
        }
        let announced = self.announced;
        let (mut delivered, unannounced): (Vec<Pending>, Vec<Pending>) =
            std::mem::take(&mut self.pending)
                .into_iter()
                .partition(|item| item.id < announced);
        // errors still waiting for a fragment, or held by one the client never heard of
        if let Some(first) = delivered.first_mut() {
            first
                .errors
                .extend(self.unplaced.drain(..).map(|(_, error)| error));
            first
                .errors
                .extend(unannounced.into_iter().flat_map(|item| item.errors));
        }
        for item in &mut delivered {
            item.errors.push(error.clone());
        }
        let delivered: Vec<(Pending, Option<&[u8]>)> =
            delivered.into_iter().map(|item| (item, None)).collect();
        self.write_part(&delivered, 0)
    }

    /// Holds new errors, and the ones whose fragment wasn't announced before, with their
    /// fragment until it goes out: one from a deferred fetch with that fetch's, any other with
    /// the one with the most of its path. The rest go to the pending fragment at `fallback`.
    fn place(&mut self, errors: Vec<GraphQLError>, origins: Vec<Option<DeferId>>, fallback: usize) {
        let candidates = self.pending.len();
        let unplaced = std::mem::take(&mut self.unplaced)
            .into_iter()
            .map(|(part, error)| (error, Some(part)));
        let origins = origins.into_iter().chain(std::iter::repeat(None));
        for (error, origin) in unplaced.chain(errors.into_iter().zip(origins)) {
            match self.owner_later(&error, origin, 0..candidates) {
                Some(owner) => self.hold(error, owner),
                None => self.pending[fallback].errors.push(error),
            }
        }
    }

    /// A later part with the `delivered` fragments. The fragments pending from `announced` on
    /// are new, the client hears about them in this part.
    fn write_part(
        &self,
        delivered: &[(Pending, Option<&[u8]>)],
        announced: usize,
    ) -> Result<Vec<u8>, ProjectionError> {
        let mut part = Vec::with_capacity(
            256 + delivered
                .iter()
                .map(|(_, output)| output.map_or(0, <[u8]>::len))
                .sum::<usize>(),
        );
        part.put_u8(b'{');
        match self.format {
            IncrementalFormat::Spec => {
                let announced = &self.pending[announced.min(self.pending.len())..];
                write_spec_part(&mut part, delivered, announced, self.plans)?;
            }
            IncrementalFormat::Legacy => write_legacy_part(&mut part, delivered, self.plans)?,
        }
        write_has_next(&mut part, self.has_next());
        part.put_u8(b'}');
        Ok(part)
    }

    /// Where the object at `path` is, with the types of the objects above it.
    fn place_of(&self, data: &Value, path: &[GraphQLErrorPathSegment]) -> Place {
        let mut types = Vec::new();
        let mut current = Some(data);
        for segment in path {
            current = match (segment, current) {
                (GraphQLErrorPathSegment::String(key), Some(Value::Object(object))) => {
                    types.push(type_name_of(object));
                    Value::object_get(object, key)
                }
                (GraphQLErrorPathSegment::Index(index), Some(Value::Array(items))) => {
                    items.get(*index)
                }
                _ => None,
            };
        }
        Place {
            path: path.to_vec(),
            types,
        }
    }

    /// Whether a group's fragment for the object `path` is under went out with data, or still
    /// can. A group that isn't active goes out with the part around it.
    fn delivers(&self, group: Option<usize>, path: &[GraphQLErrorPathSegment]) -> bool {
        let active = std::iter::successors(group, |group| self.plans.groups[*group].parent)
            .find(|group| self.deliveries[*group].is_some());
        match active {
            // the primary part
            None => true,
            Some(group) => self.ended(group, path).unwrap_or(true),
        }
    }

    /// Whether a group's fragment for the object `path` is under went out with data, `None`
    /// while it can still go out.
    fn ended(&self, group: usize, path: &[GraphQLErrorPathSegment]) -> Option<bool> {
        let anchor = object_path(path, self.plans.groups[group].depth);
        if let Some((_, _, delivered)) = self
            .finished
            .iter()
            .find(|(finished, path, _)| *finished == group && path == anchor)
        {
            return Some(*delivered);
        }
        if self
            .pending
            .iter()
            .any(|item| item.group == group && item.path == anchor)
        {
            return None;
        }
        // not announced. It still can be while the fragment it's nested in hasn't gone out,
        // after that it never is: its object isn't there, or isn't its type
        match self.deliveries[group].as_ref().and_then(|d| d.parent) {
            Some(parent) => self.ended(parent, path).map(|_| false),
            None => Some(false),
        }
    }

    /// Whether another part delivers a shared field at `place`: one that has it with these
    /// variables, on objects of these types, and that went out with data or still can.
    fn delivered(&self, field: &SharedField, place: &Place) -> bool {
        field.when.is_empty()
            || field.when.iter().any(|selected| {
                selected.conditions.iter().all(|c| c.pass(self.variables))
                    && selected.types.iter().all(|(depth, condition)| {
                        // no type to go by, so it may well be delivered
                        place
                            .types
                            .get(*depth)
                            .and_then(Option::as_deref)
                            .is_none_or(|type_name| self.type_matches(type_name, condition))
                    })
                    && self.delivers(selected.owner, &place.path)
            })
    }

    /// Whether `output` has a null on a `shared` field where `data` has a value: a null
    /// projection put in, on a field another part delivers too. A null that's in the data is
    /// fine, the other part gets the same one. `type_name` is the type of the object `data` is,
    /// or is in, for when it has no `__typename`. `place` is where `data` is.
    fn nulls_shared(
        &self,
        output: &sonic_rs::Value,
        data: &Value,
        shared: &Shared,
        type_name: &str,
        place: &mut Place,
    ) -> bool {
        if output.is_null() {
            return !data.is_null();
        }
        if let (Some(output), Value::Object(data)) = (output.as_object(), data) {
            let type_name = type_name_of(data)
                .or_else(|| Some(type_name.to_string()).filter(|name| !name.is_empty()));
            place.types.push(type_name);
            let found = output.iter().any(|(key, value)| {
                let Some(field) = shared.0.get(key) else {
                    return false;
                };
                let Some(data) = Value::object_get(data, key) else {
                    return false;
                };
                place
                    .path
                    .push(GraphQLErrorPathSegment::String(key.to_string()));
                let found = self.delivered(field, place)
                    && self.nulls_shared(value, data, &field.fields, &field.output_type, place);
                place.path.pop();
                found
            });
            place.types.pop();
            return found;
        }
        if let (Some(output), Value::Array(data)) = (output.as_array(), data) {
            return output
                .iter()
                .zip(data)
                .enumerate()
                .any(|(index, (value, data))| {
                    place.path.push(GraphQLErrorPathSegment::Index(index));
                    let found = self.nulls_shared(value, data, shared, type_name, place);
                    place.path.pop();
                    found
                });
        }
        false
    }

    /// Projects the pending fragment at `index` into `buffer`, and announces the fragments nested
    /// in it. Where its data is in `buffer`, `None` when it went null.
    fn project_pending(
        &mut self,
        index: usize,
        data: &Value,
        buffer: &mut Vec<u8>,
    ) -> Result<Option<Range<usize>>, ProjectionError> {
        let group = self.pending[index].group;
        let Some(value) = value_at(data, &self.pending[index].path) else {
            return Ok(None);
        };
        let start = buffer.len() + DATA_PREFIX.len();
        let projected = project_data(
            value,
            Vec::new(),
            &self.pending[index].type_name,
            &self.plans.groups[group].plan,
            self.variables,
            std::mem::take(buffer),
            self.schema,
        )?;
        *buffer = projected.buffer;
        if &buffer[start..] == b"null" {
            return Ok(None);
        }
        let output = start..buffer.len();
        // whether projection put in a null, one that could land on a field another part
        // delivers too
        let shared = &self.plans.groups[group].shared;
        let nulled = projected.nulled && !shared.0.is_empty();
        let children = self.children_of(Some(group));
        if !nulled && children.is_empty() {
            return Ok(Some(output));
        }

        // parsed only when it's needed: to find where the nulls are, and to find the fragments
        // nested in it
        let parsed: sonic_rs::Value = read_back(&buffer[output.clone()])?;
        // A null that bubbled up to a field another part delivers too, like `book` in
        // `{ book { ... } ... @defer { book { nonNull } } }`, can't replace it: the null stops at
        // the fragment, which goes out with its errors and no data.
        if nulled && {
            let item = &self.pending[index];
            let mut place = self.place_of(data, &item.path);
            self.nulls_shared(&parsed, value, shared, &item.type_name, &mut place)
        } {
            return Ok(None);
        }
        if children.is_empty() {
            return Ok(Some(output));
        }

        let first_child = self.pending.len();
        let anchors = self.anchors_of(&children, value, &self.pending[index].type_name);
        for (child, anchor) in anchors {
            if !alive_in_output(&parsed, &anchor.path) {
                continue;
            }
            let mut path = self.pending[index].path.clone();
            path.extend(anchor.path);
            self.push_pending(child, path, anchor.type_name);
        }
        // hand errors down to the nested fragments they belong to
        for mut error in std::mem::take(&mut self.pending[index].errors) {
            let floor = error
                .path
                .as_ref()
                .and_then(|path| self.matched(&self.pending[index], &path.segments))
                .unwrap_or(0);
            match self.owner(&error, first_child..self.pending.len(), floor) {
                Some((child, matched)) => {
                    truncate_path(&mut error, matched);
                    self.pending[child].errors.push(error);
                }
                None => self.pending[index].errors.push(error),
            }
        }
        Ok(Some(output))
    }
}

/// The keys of an `incrementalSpec=v0.2` part after `{`. `delivered` are the fragments going out
/// with their data, `None` for one that went null.
fn write_spec_part(
    part: &mut Vec<u8>,
    delivered: &[(Pending, Option<&[u8]>)],
    announced: &[Pending],
    plans: &DeferPlans,
) -> Result<(), ProjectionError> {
    if !announced.is_empty() {
        write_pending_key(part, announced, plans);
        part.put_u8(b',');
    }
    // a fragment whose fields all went out before has nothing to send, it only completes
    let mut incremental = delivered
        .iter()
        .filter_map(|(item, output)| {
            output
                .filter(|output| !item.errors.is_empty() || *output != b"{}")
                .map(|output| (item, output))
        })
        .peekable();
    if incremental.peek().is_some() {
        part.put_slice(b"\"incremental\":[");
        for (n, (item, output)) in incremental.enumerate() {
            if n > 0 {
                part.put_u8(b',');
            }
            part.put_slice(b"{\"id\":");
            write_id(part, item.id);
            part.put_slice(b",\"data\":");
            part.put_slice(output);
            if !item.errors.is_empty() {
                write_errors(part, &item.errors)?;
            }
            part.put_u8(b'}');
        }
        part.put_slice(b"],");
    }
    part.put_slice(b"\"completed\":[");
    for (n, (item, output)) in delivered.iter().enumerate() {
        if n > 0 {
            part.put_u8(b',');
        }
        part.put_slice(b"{\"id\":");
        write_id(part, item.id);
        if output.is_none() {
            write_null_errors(part, item)?;
        }
        part.put_u8(b'}');
    }
    part.put_u8(b']');
    Ok(())
}

/// The keys of a `deferSpec=20220824` part after `{`, see [`write_spec_part`].
fn write_legacy_part(
    part: &mut Vec<u8>,
    delivered: &[(Pending, Option<&[u8]>)],
    plans: &DeferPlans,
) -> Result<(), ProjectionError> {
    part.put_slice(b"\"incremental\":[");
    for (n, (item, output)) in delivered.iter().enumerate() {
        if n > 0 {
            part.put_u8(b',');
        }
        part.put_slice(b"{\"data\":");
        part.put_slice(output.unwrap_or(b"null"));
        part.put_slice(b",\"path\":");
        write_path(part, &item.path)?;
        write_label(part, plans, item);
        if output.is_none() {
            write_null_errors(part, item)?;
        } else if !item.errors.is_empty() {
            write_errors(part, &item.errors)?;
        }
        part.put_u8(b'}');
    }
    part.put_u8(b']');
    Ok(())
}

fn truncate_path(error: &mut GraphQLError, matched: usize) {
    if matched == 0 {
        error.path = None;
    } else if let Some(path) = error.path.as_mut() {
        path.segments.truncate(matched);
    }
}

/// Parses output projection just wrote. It's always valid JSON, failing here is a bug.
fn read_back(output: &[u8]) -> Result<sonic_rs::Value, ProjectionError> {
    sonic_rs::from_slice(output).map_err(|err| ProjectionError::ReadBackFailure(err.to_string()))
}

/// The value at a response path, `None` if anything on the way is missing.
fn value_at<'a, 'v>(
    data: &'a Value<'v>,
    path: &[GraphQLErrorPathSegment],
) -> Option<&'a Value<'v>> {
    let mut current = data;
    for segment in path {
        current = match (segment, current) {
            (GraphQLErrorPathSegment::String(key), Value::Object(object)) => {
                Value::object_get(object, key)?
            }
            (GraphQLErrorPathSegment::Index(index), Value::Array(items)) => items.get(*index)?,
            _ => return None,
        };
    }
    Some(current)
}

/// The errors of a fragment that went null, a generic one when it has none of its own.
fn write_null_errors(buffer: &mut Vec<u8>, item: &Pending) -> Result<(), ProjectionError> {
    if !item.errors.is_empty() {
        return write_errors(buffer, &item.errors);
    }
    let mut error = GraphQLError::from(GENERIC_NULL_ERROR);
    error.path = Some(GraphQLErrorPath {
        segments: item.path.clone(),
    });
    write_errors(buffer, &[error])
}

fn write_label(buffer: &mut Vec<u8>, plans: &DeferPlans, item: &Pending) {
    if let Some(label) = &plans.groups[item.group].marker.label {
        buffer.put_slice(b",\"label\":");
        write_and_escape_string(buffer, label);
    }
}

fn write_id(buffer: &mut Vec<u8>, id: usize) {
    // writing to a `Vec` can't fail
    let _ = write!(buffer, "\"{id}\"");
}

fn write_path(
    buffer: &mut Vec<u8>,
    path: &[GraphQLErrorPathSegment],
) -> Result<(), ProjectionError> {
    let serialized = sonic_rs::to_vec(path)
        .map_err(|e| ProjectionError::ErrorsSerializationFailure(e.to_string()))?;
    buffer.put_slice(&serialized);
    Ok(())
}

fn write_pending(buffer: &mut Vec<u8>, pending: &[Pending], plans: &DeferPlans) {
    if pending.is_empty() {
        return;
    }
    buffer.put_u8(b',');
    write_pending_key(buffer, pending, plans);
}

fn write_pending_key(buffer: &mut Vec<u8>, pending: &[Pending], plans: &DeferPlans) {
    buffer.put_slice(b"\"pending\":[");
    for (n, item) in pending.iter().enumerate() {
        if n > 0 {
            buffer.put_u8(b',');
        }
        buffer.put_slice(b"{\"id\":");
        write_id(buffer, item.id);
        buffer.put_slice(b",\"path\":");
        // path segments always serialize
        let _ = write_path(buffer, &item.path);
        write_label(buffer, plans, item);
        buffer.put_u8(b'}');
    }
    buffer.put_u8(b']');
}

fn write_has_next(buffer: &mut Vec<u8>, has_next: bool) {
    buffer.put_slice(b",\"hasNext\":");
    buffer.put_slice(if has_next { b"true" } else { b"false" });
}

#[cfg(test)]
#[path = "defer_tests.rs"]
mod tests;
