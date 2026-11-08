//! Builds a [`IndexedResponsePlan`] from a query plan and the projection plan of its operation.
//!
//! # Positions
//!
//! Every object in the response sits at a position: a response path from `data`, with the list
//! steps and type conditions left out. `users.@.reviews.@.product` is the position
//! `users.reviews.product`. Every object at one position gets the same layout: one field index per
//! response key that any fetch writes there, in the order the keys were first seen.
//!
//! One parsed entity can land in several positions: a `BatchFetch` alias with three paths parses
//! each entity once and copies it into the entity targets of all three. So those positions are unioned,
//! and so are their children with the same key, all the way down. After that, an entity's block
//! fits every entity target it is copied into, cell for cell, and its nested objects can be shared by id.
//!
//! Keys that only the client asks for (no fetch writes them) get no field index. The writer reads them
//! as missing, which is what they are.
//!
//! Nothing here needs the schema: the layouts come from the fetch documents and the plan's paths.

use std::sync::Arc;

use ahash::{HashMap as AHashMap, HashSet as AHashSet};
use graphql_tools::parser::query as gql;

use crate::executor::introspection::schema::SchemaMetadata;
use crate::executor::projection::plan::{Condition, ConditionId, FieldRecord, ProjectionPlan};
use crate::query_planner::ast::operation::OperationDefinition;
use crate::query_planner::ast::requires::{
    RequiresSelection, RequiresSelectionSet, RequiresSelectionSetRef,
};
use crate::query_planner::planner::plan_nodes::{
    CustomScalarPaths, EntityReuse, FetchNode, FetchNodePathSegment, FetchRewrite, PathSegment,
    PlanNode, QueryPlan, TypeCondition,
};
use crate::query_planner::state::supergraph_state::OperationKind;

/// No field index: the key isn't written at this position.
pub const NO_FIELD: u16 = u16::MAX;
/// A shape node without an object under it.
pub const NO_OBJECT: u32 = u32::MAX;

/// The fetch id, in [`IndexedResponsePlan::fetches`], of the data the router has before the plan runs (see
/// [`Seed`]). It's read into cells like a root fetch's response.
pub const SEED_FETCH: i64 = i64::MIN;

/// Data the router has before the plan runs, read into the root first
#[derive(Clone, Copy)]
pub enum Seed<'a> {
    None,
    /// Data the router has for an operation's fields: what it resolved for the introspection
    /// part, or what a plugin's `on_execute` put in
    Operation(&'a OperationDefinition),
    /// The subscription's own fetch: each event of its stream, before the plan resolves the
    /// event's entities
    Subscription(&'a FetchNode),
}

/// Field indexes are `u16`s and `NO_FIELD` is taken: no layout can be wider.
const MAX_WIDTH: usize = NO_FIELD as usize - 1;

pub struct IndexedResponsePlan {
    /// The projection plan this was compiled with. A request with another one can't use it.
    pub projection: Arc<ProjectionPlan>,
    pub layouts: Box<[ObjectLayout]>,
    pub root_layout: u32,
    /// For each field of the projection plan, its field index in the layout of the object it's read
    /// from, and the layout of the objects under it.
    pub fields: Box<[OutputField]>,
    /// Whether the root fields can take the writer's fast loop, see [`OutputField::SIMPLE`]
    pub root_simple: bool,
    /// For each field of the projection plan, its type in the schema when the object it's read
    /// from is of a known object type. Saves the writer two schema lookups per object for a
    /// type condition on the field's value.
    pub field_types: Box<[Option<Box<str>>]>,
    pub fetches: AHashMap<i64, FetchProgram>,
}

#[derive(Clone, Copy, Debug)]
pub struct OutputField {
    pub field_index: u16,
    /// `"key":` padded to a fixed window, so the writer copies it with one fixed-size store.
    /// `key_len` is 0 when the key doesn't fit, then the writer writes it the slow way.
    pub key_len: u8,
    /// [`OutputField::SIMPLE`]
    pub flags: u8,
    pub key: [u8; KEY_WINDOW],
}

/// 24 beat 16 and 32 on `kamil-slot-projection`
pub const KEY_WINDOW: usize = 24;

impl OutputField {
    /// The field's children can take the writer's fast loop: none of them has a type guard,
    /// a condition or is `__typename`, and every key fits the window.
    pub const SIMPLE: u8 = 1;

    fn new(field_index: u16, key: &str) -> Self {
        let mut window = [0; KEY_WINDOW];
        let len = key.len() + 3;
        if len > KEY_WINDOW {
            return Self {
                field_index,
                key_len: 0,
                flags: 0,
                key: window,
            };
        }
        window[0] = b'"';
        window[1..=key.len()].copy_from_slice(key.as_bytes());
        window[key.len() + 1] = b'"';
        window[key.len() + 2] = b':';
        Self {
            field_index,
            key_len: len as u8,
            flags: 0,
            key: window,
        }
    }
}

#[derive(Debug, Default)]
pub struct ObjectLayout {
    /// The response key of each field index
    pub keys: Vec<Box<str>>,
    pub typename: u16,
}

impl ObjectLayout {
    #[inline]
    pub fn width(&self) -> usize {
        self.keys.len()
    }

    pub fn field_index(&self, key: &str) -> u16 {
        self.keys
            .iter()
            .position(|k| &**k == key)
            .map_or(NO_FIELD, |field_index| field_index as u16)
    }
}

pub struct FetchProgram {
    pub shape: FetchShape,
    /// The fields `data` keeps entities under, one per alias. Empty for a root fetch.
    pub aliases: Vec<AliasProgram>,
    /// Where a root re-entry fetch merges its `data`
    pub merge_path: Option<PathProgram>,
}

pub struct AliasProgram {
    /// The field index of the alias in the layout of `data`
    pub field_index: u16,
    pub paths: Vec<PathProgram>,
    pub requires: Requires,
    /// Where entities of this alias may already be in the response, when entity reuse is on
    pub reuse: Option<ReuseProgram>,
    /// The `__typename` an entity target's representation says instead of its own (planner input
    /// rewrites), applied in order
    pub typename: Box<[TypenameRewrite]>,
}

/// An entity sent as another type: the planner's input rewrite that sets `__typename` of an
/// `@interfaceObject` (or interface) entity to the interface. It applies when the entity target has a
/// `__typename` that is one of `types` (or implements one).
#[derive(Debug, Clone)]
pub struct TypenameRewrite {
    pub types: Box<[Box<str>]>,
    pub to: Box<str>,
}

/// A field the fetch aliases and the response has under its own key: the planner's output
/// rewrite for fields whose types differ between subgraphs. `parent` are the keys from `data` to
/// the object, as the fetch document has them.
struct Rename {
    parent: Vec<Box<str>>,
    wire: Box<str>,
    to: Box<str>,
}

/// The input rewrites of an entity fetch, as [`TypenameRewrite`]s: the only kind the planner
/// makes. A fetch merged from others can have one per interface.
fn typename_rewrite(
    rewrites: Option<&[FetchRewrite]>,
) -> Result<Box<[TypenameRewrite]>, Unsupported> {
    let mut found = Vec::new();
    for rewrite in rewrites.into_iter().flatten() {
        match rewrite {
            FetchRewrite::ValueSetter(setter) => match setter.path.as_slice() {
                [FetchNodePathSegment::TypenameEquals(types), FetchNodePathSegment::Key(key)]
                    if key == "__typename" =>
                {
                    found.push(TypenameRewrite {
                        types: types.iter().map(|name| name.as_str().into()).collect(),
                        to: setter.set_value_to.as_str().into(),
                    })
                }
                _ => return Err(Unsupported("an input rewrite of this kind")),
            },
            _ => return Err(Unsupported("an input rewrite of this kind")),
        }
    }
    Ok(found.into())
}

/// The output rewrites of a fetch as [`Rename`]s. `under` is where they apply: `data` for a root
/// fetch, the entity alias for an entity fetch. Type conditions on the way don't matter: a
/// shape object has the fields of all its types.
fn renames(rewrites: Option<&[FetchRewrite]>, under: &[&str]) -> Result<Vec<Rename>, Unsupported> {
    let mut out = Vec::new();
    for rewrite in rewrites.into_iter().flatten() {
        let FetchRewrite::KeyRenamer(renamer) = rewrite else {
            return Err(Unsupported("an output rewrite of this kind"));
        };
        let mut keys: Vec<Box<str>> = under.iter().map(|key| (*key).into()).collect();
        for segment in &renamer.path {
            match segment {
                FetchNodePathSegment::Key(key) => keys.push(key.as_str().into()),
                FetchNodePathSegment::TypenameEquals(_) => {}
                FetchNodePathSegment::Parent => {
                    return Err(Unsupported("an output rewrite of this kind"))
                }
            }
        }
        let Some(wire) = keys.pop() else {
            return Err(Unsupported("an output rewrite of this kind"));
        };
        out.push(Rename {
            parent: keys,
            wire,
            to: renamer.rename_key_to.as_str().into(),
        });
    }
    Ok(out)
}

/// An alias's planner `EntityReuse`, resolved to field indexes. An entity target whose representation matches an
/// object at one of the sources is the same entity with the same inputs: the copied fields go
/// from that object's cells into the entity target's, and the entity target isn't sent.
pub struct ReuseProgram {
    /// The layout of the alias's entity targets
    pub target_layout: u32,
    /// The field index of each copied field in the entity targets
    pub copy: Box<[u16]>,
    pub sources: Vec<ReuseSource>,
}

pub struct ReuseSource {
    pub path: PathProgram,
    /// The alias's representation, read at the source's position
    pub requires: Requires,
    /// The field index of each copied field at the source, in `ReuseProgram::copy` order
    pub copy: Box<[u16]>,
}

pub type PathProgram = Box<[Step]>;

#[derive(Debug)]
pub enum Step {
    /// The field index of the field
    Field(u16),
    List,
    TypeCondition(Box<TypeCondition>),
}

/// A `@requires` selection, with the field indexes it reads resolved.
#[derive(Debug, Default)]
pub struct Requires {
    pub nodes: Vec<RequiresNode>,
    pub root: std::ops::Range<u32>,
    /// Every field the selection may read at each level, inline fragments flattened: what a
    /// fingerprint hashes. Whether a fragment applies follows from `__typename`, which the
    /// fingerprint hashes too, so it doesn't need to decide it.
    pub reads: Vec<Read>,
    pub reads_root: std::ops::Range<u32>,
}

#[derive(Debug, Clone)]
pub struct Read {
    pub by_name: u16,
    pub by_alias: u16,
    /// Its own reads, empty for a leaf
    pub children: std::ops::Range<u32>,
}

impl Requires {
    fn flatten(&mut self, selections: std::ops::Range<u32>) -> std::ops::Range<u32> {
        let mut level = Vec::new();
        self.collect_level(selections, &mut level);
        let start = self.reads.len() as u32;
        let end = start + level.len() as u32;
        self.reads
            .extend(level.iter().map(|(by_name, by_alias, _)| Read {
                by_name: *by_name,
                by_alias: *by_alias,
                children: 0..0,
            }));
        for (offset, (_, _, children)) in level.into_iter().enumerate() {
            if !children.is_empty() {
                let flat = self.flatten(children);
                self.reads[start as usize + offset].children = flat;
            }
        }
        start..end
    }

    #[allow(clippy::type_complexity)]
    fn collect_level(
        &self,
        selections: std::ops::Range<u32>,
        out: &mut Vec<(u16, u16, std::ops::Range<u32>)>,
    ) {
        for node in &self.nodes[selections.start as usize..selections.end as usize] {
            match node {
                RequiresNode::Field {
                    response_key,
                    by_name,
                    by_alias,
                    children,
                } => {
                    // `__typename` is read from the typename cell, which is hashed anyway
                    if &**response_key != "__typename" {
                        out.push((*by_name, *by_alias, children.clone()));
                    }
                }
                RequiresNode::InlineFragment { children, .. } => {
                    self.collect_level(children.clone(), out)
                }
            }
        }
    }
}

#[derive(Debug)]
pub enum RequiresNode {
    Field {
        /// The key written to the representation
        response_key: Box<str>,
        /// The field index under the field's name, then the one under its alias (like
        /// `project_requires`, which looks the name up first)
        by_name: u16,
        by_alias: u16,
        children: std::ops::Range<u32>,
    },
    InlineFragment {
        type_condition: Box<str>,
        children: std::ops::Range<u32>,
    },
}

/// The fetch's query, as the parser reads it: like `ResponseShape`, but every node knows its
/// field index.
#[derive(Debug, Default)]
pub struct FetchShape {
    pub objects: Vec<ShapeObject>,
    pub nodes: Vec<ShapeNode>,
    pub text: Vec<u8>,
    /// Nodes whose value goes under another key than the one the subgraph sends (see
    /// [`Rename`]), by node index
    renamed: AHashMap<u32, Box<str>>,
    pub verifies: Vec<Verify>,
    pub verify_objects: Vec<VerifyObject>,
    /// The schema field name of each node, a range in `text`, for a walk over the fetch's data
    /// (see `SubgraphData`). Empty when the key is a different field depending on the type.
    pub names: Vec<(u32, u32)>,
    /// Per object, the type its fields are on when the query says it: every selection is in a
    /// fragment on that one type, like the items of `_entities`. A range in `text`, empty when
    /// it doesn't say.
    pub object_types: Vec<(u32, u32)>,
}

#[derive(Debug, Clone, Copy)]
pub struct ShapeObject {
    pub layout: u32,
    /// Its fields' nodes, `start..end`
    pub nodes: (u32, u32),
    position: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct ShapeNode {
    /// First 8 bytes of the quoted key, zero padded
    pub prefix: u64,
    /// `"key":`, zero padded, when it fits in 16 bytes; 0 when it doesn't
    pub key_colon: u128,

    pub key_start: u32,
    pub key_len: u32,
    pub field_index: u16,
    /// The shape object of its value, `NO_OBJECT` for a leaf
    pub object: u32,
    /// A custom scalar, kept as the JSON it came as
    pub opaque_json: bool,
    /// A subtree the client gets exactly as the subgraph sends it, see [`Verify`]. `NO_OBJECT`
    /// when it isn't one.
    pub verify: u32,
}

/// A value the parser may keep as one raw range, for the writer to copy as it is. The client
/// selects exactly what this one fetch selects there, in the same order, with no conditions,
/// no `__typename` and no other fetch writing below it. So when the value has exactly the
/// selected keys, in order, and no null where the type says non-null, the client's bytes are
/// the subgraph's. The parser checks that as it reads it, and reads it the normal way when it
/// doesn't hold.
#[derive(Debug, Clone)]
pub struct Verify {
    /// Nullability of the value, one byte per type level (see `ShapeFlags`)
    pub shape: Box<[u8]>,
    /// The object it expects, an index into `FetchShape::verify_objects`, or `NO_OBJECT` for a
    /// scalar
    pub object: u32,
    /// For an enum: the values a string may be
    pub enum_values: Option<Box<[Box<str>]>>,
    /// A custom scalar, kept as the JSON it came as: any value but a null where it's non-null
    pub any: bool,
}

#[derive(Debug, Clone, Default)]
pub struct VerifyObject {
    /// The fields, in order: the shape node to match the key with, and what its value must be
    pub fields: Vec<(u32, Verify)>,
}

impl FetchShape {
    /// A field's name in the schema, see [`FetchShape::names`]
    pub fn name(&self, node: usize) -> &str {
        let (start, len) = self.names[node];
        std::str::from_utf8(&self.text[start as usize..(start + len) as usize]).unwrap_or("")
    }

    /// The type of an object's fields, see [`FetchShape::object_types`]
    pub fn object_type(&self, object: u32) -> Option<&str> {
        let (start, len) = self.object_types[object as usize];
        (len > 0)
            .then(|| std::str::from_utf8(&self.text[start as usize..(start + len) as usize]).ok())
            .flatten()
    }

    #[inline]
    pub fn key(&self, node: &ShapeNode) -> &[u8] {
        &self.text[node.key_start as usize..(node.key_start + node.key_len) as usize]
    }
}

// ---------------------------------------------------------------------------------------------

/// Union-find over positions, with the children of each class.
#[derive(Default)]
struct Positions {
    parent: Vec<u32>,
    /// Children by key, kept on the class representative
    children: Vec<Vec<(Box<str>, u32)>>,
    /// Keys written at the position, in order, kept on the representative
    keys: Vec<Vec<Box<str>>>,
}

impl Positions {
    fn new_position(&mut self) -> u32 {
        let id = self.parent.len() as u32;
        self.parent.push(id);
        self.children.push(Vec::new());
        self.keys.push(Vec::new());
        id
    }

    fn find(&mut self, mut p: u32) -> u32 {
        while self.parent[p as usize] != p {
            let grand = self.parent[self.parent[p as usize] as usize];
            self.parent[p as usize] = grand;
            p = grand;
        }
        p
    }

    fn child(&mut self, p: u32, key: &str) -> u32 {
        let p = self.find(p);
        if let Some((_, c)) = self.children[p as usize].iter().find(|(k, _)| &**k == key) {
            return *c;
        }
        let c = self.new_position();
        self.children[p as usize].push((key.into(), c));
        c
    }

    fn existing_child(&mut self, p: u32, key: &str) -> Option<u32> {
        let p = self.find(p);
        self.children[p as usize]
            .iter()
            .find(|(k, _)| &**k == key)
            .map(|(_, c)| *c)
    }

    fn add_key(&mut self, p: u32, key: &str) {
        let p = self.find(p) as usize;
        if !self.keys[p].iter().any(|k| &**k == key) {
            self.keys[p].push(key.into());
        }
    }

    fn union(&mut self, a: u32, b: u32) {
        let mut work = vec![(a, b)];
        while let Some((a, b)) = work.pop() {
            let (a, b) = (self.find(a), self.find(b));
            if a == b {
                continue;
            }
            self.parent[b as usize] = a;
            for key in std::mem::take(&mut self.keys[b as usize]) {
                if !self.keys[a as usize].contains(&key) {
                    self.keys[a as usize].push(key);
                }
            }
            for (key, child) in std::mem::take(&mut self.children[b as usize]) {
                match self.children[a as usize].iter().find(|(k, _)| *k == key) {
                    Some((_, existing)) => work.push((*existing, child)),
                    None => self.children[a as usize].push((key, child)),
                }
            }
        }
    }

    /// The position at the end of a plan path. List steps and type conditions don't move.
    fn at_path(&mut self, root: u32, path: &[PathSegment]) -> u32 {
        let mut p = root;
        for segment in path {
            if let PathSegment::Field(key) = segment {
                p = self.child(p, key);
            }
        }
        p
    }
}

/// The response keys of a copy selection, when it's only leaf fields (inline fragments of the
/// entity's type flattened).
fn copy_keys<'s>(selections: RequiresSelectionSetRef<'s>, keys: &mut Vec<&'s str>) -> Option<()> {
    for selection in selections.iter() {
        match selection {
            RequiresSelection::Field {
                name,
                alias,
                selections,
                ..
            } => {
                if !selections.is_empty() {
                    return None;
                }
                keys.push(alias.unwrap_or(name));
            }
            RequiresSelection::InlineFragment { selections, .. } => copy_keys(selections, keys)?,
            RequiresSelection::FragmentSpread(_) => return None,
        }
    }
    Some(())
}

/// Why a plan can't use the indexed pipeline. Only for logs and tests.
#[derive(Debug)]
pub struct Unsupported(pub &'static str);

struct Compiler<'p> {
    positions: Positions,
    root: u32,
    fetches: Vec<(i64, PendingFetch<'p>)>,
    /// `traffic_shaping.experimental_entity_reuse`
    entity_reuse: bool,
}

/// An entity alias of a fetch: its key in `data`, the position of its items, its merge paths
/// and its requires selection.
type PendingAlias<'p> = (
    Box<str>,
    u32,
    Vec<&'p [PathSegment]>,
    &'p RequiresSelectionSet,
    Option<&'p EntityReuse>,
    Box<[TypenameRewrite]>,
);

/// An entity alias as the plan has it: its key in `data`, merge paths, requires selection and
/// where its entities may already be.
type PlanAlias<'p> = (
    Box<str>,
    Vec<&'p [PathSegment]>,
    &'p RequiresSelectionSet,
    Option<&'p EntityReuse>,
    Box<[TypenameRewrite]>,
);

struct PendingFetch<'p> {
    shape: FetchShape,
    aliases: Vec<PendingAlias<'p>>,
    merge_path: Option<&'p [PathSegment]>,
}

impl IndexedResponsePlan {
    pub fn compile(
        plan: &QueryPlan,
        projection: &Arc<ProjectionPlan>,
        schema: &SchemaMetadata,
        root_type: &str,
        seed: Seed,
        entity_reuse: bool,
    ) -> Result<IndexedResponsePlan, Unsupported> {
        let mut compiler = Compiler {
            positions: Positions::default(),
            root: 0,
            fetches: Vec::new(),
            entity_reuse,
        };
        compiler.root = compiler.positions.new_position();
        if let Some(node) = &plan.node {
            compiler.node(node)?;
        }
        match seed {
            Seed::None => {}
            Seed::Operation(operation) => compiler.document(
                SEED_FETCH,
                &operation.to_string(),
                None,
                None,
                None,
                Vec::new(),
            )?,
            Seed::Subscription(fetch) => compiler.document(
                SEED_FETCH,
                &fetch.operation.document_str,
                fetch.custom_scalar_paths.as_ref(),
                None,
                None,
                renames(fetch.output_rewrites.as_deref(), &[])?,
            )?,
        }
        compiler.finish(projection, schema, root_type)
    }
}

impl<'p> Compiler<'p> {
    fn node(&mut self, node: &'p PlanNode) -> Result<(), Unsupported> {
        match node {
            PlanNode::Sequence(seq) => seq.nodes.iter().try_for_each(|n| self.node(n)),
            PlanNode::Parallel(par) => par.nodes.iter().try_for_each(|n| self.node(n)),
            PlanNode::Fetch(fetch) => self.fetch(fetch, None),
            PlanNode::Flatten(flatten) => match flatten.node.as_ref() {
                PlanNode::Fetch(fetch) => self.fetch(fetch, Some(flatten.path.as_slice())),
                // the other pipeline doesn't run these either
                _ => Ok(()),
            },
            PlanNode::BatchFetch(batch) => {
                if matches!(batch.operation_kind, Some(OperationKind::Subscription)) {
                    return Err(Unsupported("a subscription"));
                }
                let mut aliases = Vec::new();
                let mut alias_renames = Vec::new();
                for alias in &batch.entity_batch.aliases {
                    alias_renames.extend(renames(
                        alias.output_rewrites.as_deref(),
                        &[alias.alias.as_str()],
                    )?);
                    aliases.push((
                        alias.alias.as_str().into(),
                        alias.merge_paths.iter().map(|p| p.as_slice()).collect(),
                        &alias.requires,
                        alias.entity_reuse.as_deref(),
                        typename_rewrite(alias.input_rewrites.as_deref())?,
                    ));
                }
                self.document(
                    batch.id,
                    &batch.operation.document_str,
                    batch.custom_scalar_paths.as_ref(),
                    Some(aliases),
                    None,
                    alias_renames,
                )
            }
            // the planner makes no other node here; the other pipeline doesn't run one either
            // (a subscription's is handled before a plan gets here)
            _ => Ok(()),
        }
    }

    fn fetch(
        &mut self,
        fetch: &'p FetchNode,
        flatten: Option<&'p [PathSegment]>,
    ) -> Result<(), Unsupported> {
        // context: one request per group of entities with the same context values, see
        // `Run::prepare_context`. Only an entity fetch has entities to group.
        if (fetch.context.is_some() || fetch.operation.context_alias_template().is_some())
            && (fetch.requires.is_none() || flatten.is_none())
        {
            return Err(Unsupported("context without entities"));
        }
        if matches!(fetch.operation_kind, Some(OperationKind::Subscription)) {
            return Err(Unsupported("a subscription"));
        }
        match (&fetch.requires, flatten) {
            (Some(requires), Some(path)) => self.document(
                fetch.id,
                &fetch.operation.document_str,
                fetch.custom_scalar_paths.as_ref(),
                Some(vec![(
                    "_entities".into(),
                    vec![path],
                    requires,
                    fetch.entity_reuse.as_deref(),
                    typename_rewrite(fetch.input_rewrites.as_deref())?,
                )]),
                None,
                renames(fetch.output_rewrites.as_deref(), &["_entities"])?,
            ),
            (Some(_), None) => Err(Unsupported("requires without a path")),
            // input rewrites change representations, and a fetch without requires has none:
            // the other pipeline doesn't use them either
            (None, merge_path) => self.document(
                fetch.id,
                &fetch.operation.document_str,
                fetch.custom_scalar_paths.as_ref(),
                None,
                merge_path,
                renames(fetch.output_rewrites.as_deref(), &[])?,
            ),
        }
    }

    /// Parses the fetch's query and builds its shape. `aliases` are the entity fields of an
    /// entity fetch, with their merge paths.
    #[allow(clippy::type_complexity)]
    fn document(
        &mut self,
        id: i64,
        document: &str,
        paths: Option<&CustomScalarPaths>,
        aliases: Option<Vec<PlanAlias<'p>>>,
        merge_path: Option<&'p [PathSegment]>,
        renames: Vec<Rename>,
    ) -> Result<(), Unsupported> {
        let paths = paths.filter(|paths| !paths.is_empty());
        if paths.is_some_and(|paths| paths.group_suffixed || paths.terminal) {
            return Err(Unsupported("custom scalar paths of this kind"));
        }
        let parsed = gql::parse_query::<&str>(document)
            .map_err(|_| Unsupported("fetch document doesn't parse"))?;
        let mut operation = None;
        let mut fragments = Vec::new();
        for definition in &parsed.definitions {
            match definition {
                gql::Definition::Operation(op) => {
                    if operation.is_some() {
                        return Err(Unsupported("several operations"));
                    }
                    operation = Some(match op {
                        gql::OperationDefinition::Query(q) => &q.selection_set,
                        gql::OperationDefinition::Mutation(m) => &m.selection_set,
                        gql::OperationDefinition::SelectionSet(s) => s,
                        // only as a seed: plan nodes of a subscription are rejected before
                        gql::OperationDefinition::Subscription(s) => &s.selection_set,
                    });
                }
                gql::Definition::Fragment(fragment) => fragments.push(fragment),
            }
        }
        let operation = operation.ok_or(Unsupported("no operation"))?;
        let mut builder = ShapeBuilder {
            fragments: &fragments,
            renames: &renames,
            trail: Vec::new(),
            shape: FetchShape::default(),
        };

        // `data` of an entity fetch is a position of its own, its keys are the aliases
        let data_position = match (&aliases, merge_path) {
            (Some(_), _) => self.positions.new_position(),
            (None, Some(path)) => self.positions.at_path(self.root, path),
            (None, None) => self.root,
        };
        builder.object(&mut self.positions, &[operation], data_position, paths)?;

        let mut pending_aliases = Vec::new();
        for (key, merge_paths, requires, reuse, typename) in aliases.into_iter().flatten() {
            let Some(items) = self.positions.existing_child(data_position, &key) else {
                return Err(Unsupported("entity alias not in the fetch document"));
            };
            for path in &merge_paths {
                let target = self.positions.at_path(self.root, path);
                self.positions.union(target, items);
            }
            pending_aliases.push((key, items, merge_paths, requires, reuse, typename));
        }
        self.fetches.push((
            id,
            PendingFetch {
                shape: builder.shape,
                aliases: pending_aliases,
                merge_path,
            },
        ));
        Ok(())
    }

    fn finish(
        mut self,
        projection: &Arc<ProjectionPlan>,
        schema: &SchemaMetadata,
        root_type: &str,
    ) -> Result<IndexedResponsePlan, Unsupported> {
        let entity_reuse = self.entity_reuse;
        // one layout per class
        let mut layout_of: AHashMap<u32, u32> = AHashMap::default();
        let mut layouts = Vec::new();
        for p in 0..self.positions.parent.len() as u32 {
            let rep = self.positions.find(p);
            if layout_of.contains_key(&rep) {
                continue;
            }
            let keys = self.positions.keys[rep as usize].clone();
            if keys.len() > MAX_WIDTH {
                return Err(Unsupported("object too wide"));
            }
            let mut layout = ObjectLayout {
                keys,
                typename: NO_FIELD,
            };
            layout.typename = layout.field_index("__typename");
            layout_of.insert(rep, layouts.len() as u32);
            layouts.push(layout);
        }
        let mut touched = AHashSet::default();
        let mut ctx = Resolved {
            positions: &mut self.positions,
            layout_of: &layout_of,
            layouts: &layouts,
            root: self.root,
            touched: &mut touched,
        };

        let root_layout = ctx.layout(self.root).expect("the root has a layout");
        let mut fetches = AHashMap::default();
        for (id, pending) in std::mem::take(&mut self.fetches) {
            let mut shape = pending.shape;
            for index in 0..shape.objects.len() {
                let layout = ctx
                    .layout(shape.objects[index].position)
                    .expect("shape objects have positions with layouts");
                shape.objects[index].layout = layout;
                let keys = &layouts[layout as usize];
                let (start, end) = shape.objects[index].nodes;
                for node_index in start..end {
                    let node = shape.nodes[node_index as usize];
                    let key = match shape.renamed.get(&node_index) {
                        Some(renamed) => renamed,
                        None => std::str::from_utf8(
                            &shape.text[node.key_start as usize + 1
                                ..(node.key_start + node.key_len) as usize - 1],
                        )
                        .expect("keys come from a str"),
                    };
                    shape.nodes[node_index as usize].field_index = keys.field_index(key);
                }
            }
            let data_layout = shape.objects[0].layout as usize;
            let aliases = pending
                .aliases
                .into_iter()
                .map(|(key, items, merge_paths, requires, reuse, typename)| {
                    let mut program = Requires::default();
                    program.root =
                        ctx.requires(&mut program, Some(items), requires.root_selections());
                    program.reads_root = program.flatten(program.root.clone());
                    let reuse = reuse
                        .filter(|_| entity_reuse)
                        .and_then(|reuse| ctx.reuse(items, requires, reuse));
                    AliasProgram {
                        field_index: layouts[data_layout].field_index(&key),
                        paths: merge_paths.iter().map(|path| ctx.path(path)).collect(),
                        requires: program,
                        reuse,
                        typename,
                    }
                })
                .collect();
            let merge_path = pending.merge_path.map(|path| ctx.path(path));
            fetches.insert(
                id,
                FetchProgram {
                    shape,
                    aliases,
                    merge_path,
                },
            );
        }

        // the projection plan's fields, by walking it along the positions
        let mut fields = vec![OutputField::new(NO_FIELD, ""); projection.field_count()];
        let mut field_types = vec![None; projection.field_count()];
        ctx.projection(
            projection,
            projection.root_range(),
            Some(self.root),
            Some(root_type),
            schema,
            &mut fields,
            &mut field_types,
        );

        let simple = |range: crate::executor::projection::plan::Range, fields: &[OutputField]| {
            projection
                .fields(range)
                .iter()
                .enumerate()
                .all(|(offset, field)| {
                    field.parent_guard.is_none()
                        && field.condition.is_none()
                        && !field.is_typename()
                        && fields[range.start as usize + offset].key_len != 0
                })
        };
        let root_simple = simple(projection.root_range(), &fields);
        for index in 0..fields.len() {
            let record = projection.field_at(index);
            if record.has_children() && simple(record.children, &fields) {
                fields[index].flags |= OutputField::SIMPLE;
            }
        }

        // subtrees the client gets as the subgraph sends them
        let mut writers: AHashMap<u32, Vec<(i64, u32)>> = AHashMap::default();
        for (id, program) in &fetches {
            for (index, object) in program.shape.objects.iter().enumerate() {
                let class = ctx.positions.find(object.position);
                writers.entry(class).or_default().push((*id, index as u32));
            }
        }
        let mut passthrough = Passthrough {
            plan: projection,
            touched: ctx.touched.clone(),
            positions: ctx.positions,
            writers: &writers,
            fetches: &mut fetches,
            field_types: &field_types,
            readers: AHashMap::default(),
        };
        passthrough.count_readers(projection.root_range(), Some(self.root));
        passthrough.walk(projection.root_range(), Some(self.root));

        Ok(IndexedResponsePlan {
            projection: Arc::clone(projection),
            layouts: layouts.into(),
            root_layout,
            fields: fields.into(),
            root_simple,
            field_types: field_types.into(),
            fetches,
        })
    }
}

/// Positions with their layouts known. Walks paths, requires and the projection plan along them,
/// without making new positions: what no fetch writes has no field index.
struct Resolved<'c> {
    positions: &'c mut Positions,
    layout_of: &'c AHashMap<u32, u32>,
    layouts: &'c [ObjectLayout],
    root: u32,
    /// Classes that paths walk through or requires read: they must stay cells, never a raw
    /// passthrough
    touched: &'c mut AHashSet<u32>,
}

impl Resolved<'_> {
    fn layout(&mut self, p: u32) -> Option<u32> {
        let rep = self.positions.find(p);
        self.layout_of.get(&rep).copied()
    }

    fn field_index(&mut self, p: Option<u32>, key: &str) -> u16 {
        p.and_then(|p| self.layout(p))
            .map_or(NO_FIELD, |l| self.layouts[l as usize].field_index(key))
    }

    fn path(&mut self, path: &[PathSegment]) -> PathProgram {
        let mut p = Some(self.root);
        let mut steps = Vec::with_capacity(path.len());
        for segment in path {
            match segment {
                PathSegment::Field(key) => {
                    steps.push(Step::Field(self.field_index(p, key)));
                    p = p.and_then(|p| self.positions.existing_child(p, key));
                    if let Some(p) = p {
                        let class = self.positions.find(p);
                        self.touched.insert(class);
                    }
                }
                PathSegment::List => steps.push(Step::List),
                PathSegment::TypeCondition(condition) => {
                    steps.push(Step::TypeCondition(condition.clone()))
                }
            }
        }
        steps.into()
    }

    /// Resolves an alias's `EntityReuse`. None when the copy isn't plain leaf fields, or no
    /// source has a field index for every one of them.
    fn reuse(
        &mut self,
        items: u32,
        requires: &RequiresSelectionSet,
        reuse: &EntityReuse,
    ) -> Option<ReuseProgram> {
        let target_layout = self.layout(items)?;
        let mut keys = Vec::new();
        copy_keys(reuse.copy.root_selections(), &mut keys)?;
        let fields_of = |layout: u32, layouts: &[ObjectLayout]| -> Option<Box<[u16]>> {
            let fields: Box<[u16]> = keys
                .iter()
                .map(|key| layouts[layout as usize].field_index(key))
                .collect();
            (!fields.contains(&NO_FIELD)).then_some(fields)
        };
        let copy = fields_of(target_layout, self.layouts)?;
        let mut sources = Vec::new();
        for source in &reuse.sources {
            let mut p = Some(self.root);
            for segment in source.as_slice() {
                if let PathSegment::Field(key) = segment {
                    p = p.and_then(|p| self.positions.existing_child(p, key));
                }
            }
            let Some(layout) = p.and_then(|p| self.layout(p)) else {
                continue;
            };
            let Some(source_copy) = fields_of(layout, self.layouts) else {
                continue;
            };
            let mut program = Requires::default();
            program.root = self.requires(&mut program, p, requires.root_selections());
            program.reads_root = program.flatten(program.root.clone());
            sources.push(ReuseSource {
                path: self.path(source.as_slice()),
                requires: program,
                copy: source_copy,
            });
        }
        (!sources.is_empty()).then_some(ReuseProgram {
            target_layout,
            copy,
            sources,
        })
    }

    fn requires(
        &mut self,
        program: &mut Requires,
        position: Option<u32>,
        selections: RequiresSelectionSetRef<'_>,
    ) -> std::ops::Range<u32> {
        let start = program.nodes.len() as u32;
        let items: Vec<_> = selections.iter().collect();
        // reserve the run, so it stays contiguous, children go after it
        for _ in &items {
            program.nodes.push(RequiresNode::InlineFragment {
                type_condition: "".into(),
                children: 0..0,
            });
        }
        for (offset, item) in items.into_iter().enumerate() {
            let node = match item {
                RequiresSelection::Field {
                    name,
                    alias,
                    selections,
                    ..
                } => {
                    let by_name = self.field_index(position, name);
                    let by_alias =
                        alias.map_or(NO_FIELD, |alias| self.field_index(position, alias));
                    // the value is read under the name first, so its objects are there
                    let child_key = if by_name != NO_FIELD {
                        name
                    } else {
                        alias.unwrap_or(name)
                    };
                    let child = position.and_then(|p| self.positions.existing_child(p, child_key));
                    if let Some(child) = child {
                        let class = self.positions.find(child);
                        self.touched.insert(class);
                    }
                    let children = if selections.is_empty() {
                        0..0
                    } else {
                        self.requires(program, child, selections)
                    };
                    RequiresNode::Field {
                        response_key: alias.unwrap_or(name).into(),
                        by_name,
                        by_alias,
                        children,
                    }
                }
                RequiresSelection::InlineFragment {
                    type_condition,
                    selections,
                    ..
                } => RequiresNode::InlineFragment {
                    type_condition: type_condition.into(),
                    children: self.requires(program, position, selections),
                },
                // the planner inlines fragments in requires
                RequiresSelection::FragmentSpread(_) => RequiresNode::InlineFragment {
                    type_condition: "".into(),
                    children: 0..0,
                },
            };
            program.nodes[start as usize + offset] = node;
        }
        start..start + selections.len() as u32
    }

    #[allow(clippy::too_many_arguments)]
    fn projection(
        &mut self,
        plan: &ProjectionPlan,
        range: crate::executor::projection::plan::Range,
        position: Option<u32>,
        parent_type: Option<&str>,
        schema: &SchemaMetadata,
        out: &mut [OutputField],
        types: &mut [Option<Box<str>>],
    ) {
        for (offset, field) in plan.fields(range).iter().enumerate() {
            let key = plan.response_key(field);
            let index = range.start as usize + offset;
            out[index] = OutputField::new(self.field_index(position, key), key);
            // only an object type is the same at runtime, an interface's object can be any of
            // its types, and so can the type of its fields
            let field_type = parent_type
                .and_then(|parent| schema.get_type_fields(parent))
                .and_then(|fields| fields.get(plan.field_name(field)))
                .map(|info| info.output_type_name.as_str());
            types[index] = field_type.map(Into::into);
            if field.has_children() {
                let child = position.and_then(|p| self.positions.existing_child(p, key));
                let child_type = field_type.filter(|name| schema.is_object_type(name));
                self.projection(plan, field.children, child, child_type, schema, out, types);
            }
        }
    }
}

/// Finds the subtrees the client gets exactly as one fetch sends them, see [`Verify`].
struct Passthrough<'c> {
    plan: &'c ProjectionPlan,
    touched: AHashSet<u32>,
    /// See `IndexedResponsePlan::field_types`
    field_types: &'c [Option<Box<str>>],
    /// How many fields of the projection plan read the objects of each class. Two fields can
    /// select different things from one position (one per type, say), and a passthrough can
    /// only give one of them what it asked for.
    readers: AHashMap<u32, u32>,
    positions: &'c mut Positions,
    /// The shape objects that write each class: fetch and object index
    writers: &'c AHashMap<u32, Vec<(i64, u32)>>,
    fetches: &'c mut AHashMap<i64, FetchProgram>,
}

impl Passthrough<'_> {
    fn count_readers(
        &mut self,
        range: crate::executor::projection::plan::Range,
        position: Option<u32>,
    ) {
        let plan = self.plan;
        for field in plan.fields(range) {
            if !field.has_children() {
                continue;
            }
            let child =
                position.and_then(|p| self.positions.existing_child(p, plan.response_key(field)));
            if let Some(child) = child {
                let class = self.positions.find(child);
                *self.readers.entry(class).or_default() += 1;
            }
            self.count_readers(field.children, child);
        }
    }

    fn walk(&mut self, range: crate::executor::projection::plan::Range, position: Option<u32>) {
        let plan = self.plan;
        for field in plan.fields(range) {
            if !field.has_children() {
                continue;
            }
            let key = plan.response_key(field);
            let child = position.and_then(|p| self.positions.existing_child(p, key));
            let Some(child) = child else {
                continue;
            };
            if let Some((id, object)) = self.single_writer(child) {
                if self.matches(field.children, child, id, object) {
                    let verify = self.verify(field, id, object);
                    let shape = &mut self.fetches.get_mut(&id).expect("a fetch").shape;
                    let index = shape.verifies.len() as u32;
                    shape.verifies.push(verify);
                    if let Some(node) = shape.nodes.iter_mut().find(|n| n.object == object) {
                        node.verify = index;
                    }
                    // the subtree is decided, but a part of it may still pass on its own when
                    // the whole doesn't
                }
            }
            self.walk(field.children, Some(child));
        }
    }

    fn single_writer(&mut self, position: u32) -> Option<(i64, u32)> {
        let class = self.positions.find(position);
        if self.touched.contains(&class) || self.readers.get(&class) != Some(&1) {
            return None;
        }
        match self.writers.get(&class)?.as_slice() {
            [single] => Some(*single),
            _ => None,
        }
    }

    /// Whether the client's fields in `range` are exactly the fields of shape object `object`,
    /// in order, and so on down.
    fn matches(
        &mut self,
        range: crate::executor::projection::plan::Range,
        position: u32,
        id: i64,
        object: u32,
    ) -> bool {
        let plan = self.plan;
        let fields = plan.fields(range);
        let shape = &self.fetches[&id].shape;
        let (start, end) = shape.objects[object as usize].nodes;
        if fields.len() != (end - start) as usize {
            return false;
        }
        let nodes: Vec<ShapeNode> = shape.nodes[start as usize..end as usize].to_vec();
        for (offset, (field, node)) in fields.iter().zip(nodes).enumerate() {
            let index = range.start as usize + offset;
            if field
                .condition
                .is_some_and(|condition| !self.checkable(condition, index))
                || field.parent_guard.is_some()
                || field.is_typename()
                || field.is_null_value()
            {
                return false;
            }
            let key = plan.response_key(field);
            let shape = &self.fetches[&id].shape;
            let node_key = &shape.key(&node);
            if &node_key[1..node_key.len() - 1] != key.as_bytes() {
                return false;
            }
            match (field.has_children(), node.object) {
                (false, NO_OBJECT) => {}
                (true, child_object) if child_object != NO_OBJECT && !node.opaque_json => {
                    let Some(child) = self.positions.existing_child(position, key) else {
                        return false;
                    };
                    if self.single_writer(child) != Some((id, child_object))
                        || !self.matches(field.children, child, id, child_object)
                    {
                        return false;
                    }
                }
                _ => return false,
            }
        }
        true
    }

    /// Whether a passthrough can still apply a field's condition: an enum check, made on the
    /// raw value, or a type condition its static type always meets (a passthrough object has
    /// no `__typename`, so its type is the field's type in the schema).
    fn checkable(&self, condition: ConditionId, index: usize) -> bool {
        match self.plan.condition(condition) {
            Condition::EnumValues(_) => true,
            Condition::FieldType(guard) => self.field_types[index]
                .as_deref()
                .is_some_and(|name| self.plan.guard_matches(guard, name)),
            _ => false,
        }
    }

    /// The enum values a field's value must be one of
    fn enum_values(&self, field: &FieldRecord) -> Option<Box<[Box<str>]>> {
        match self.plan.condition(field.condition?) {
            Condition::EnumValues(set) => Some(
                self.plan
                    .members(set)
                    .iter()
                    .map(|member| self.plan.text(*member).into())
                    .collect(),
            ),
            _ => None,
        }
    }

    fn verify(&mut self, field: &FieldRecord, id: i64, object: u32) -> Verify {
        let plan = self.plan;
        let fields: Vec<FieldRecord> = plan.fields(field.children).to_vec();
        let (start, _) = self.fetches[&id].shape.objects[object as usize].nodes;
        let mut verify_object = VerifyObject::default();
        for (offset, child) in fields.iter().enumerate() {
            let node = start + offset as u32;
            let child_node = self.fetches[&id].shape.nodes[node as usize];
            let verify = if child.has_children() {
                self.verify(child, id, child_node.object)
            } else {
                Verify {
                    shape: plan.shape(child.nullability()).into(),
                    object: NO_OBJECT,
                    enum_values: self.enum_values(child),
                    any: child_node.opaque_json,
                }
            };
            verify_object.fields.push((node, verify));
        }
        let shape = &mut self.fetches.get_mut(&id).expect("a fetch").shape;
        let index = shape.verify_objects.len() as u32;
        shape.verify_objects.push(verify_object);
        Verify {
            shape: plan.shape(field.nullability()).into(),
            object: index,
            enum_values: None,
            any: false,
        }
    }
}

/// A response key with the field's name (`None` when fragments put different fields under it)
/// and the selection sets under it
pub(crate) type Collected<'d, 'a> = (
    &'a str,
    Option<&'a str>,
    Vec<&'d gql::SelectionSet<'a, &'a str>>,
);

/// The fields `set` selects, by response key, through its fragments: each key's field name
/// (`None` when fragments put different fields under it) and the selection sets under it
pub(crate) fn collect_fields<'d, 'a>(
    fragments: &'d [&'d gql::FragmentDefinition<'a, &'a str>],
    set: &'d gql::SelectionSet<'a, &'a str>,
    out: &mut Vec<Collected<'d, 'a>>,
) -> Result<(), Unsupported> {
    for item in &set.items {
        match item {
            gql::Selection::Field(field) => {
                let key = field.alias.unwrap_or(field.name);
                let index = match out.iter().position(|(k, _, _)| *k == key) {
                    Some(index) => {
                        // another field under the same key: no one name for it
                        if out[index].1 != Some(field.name) {
                            out[index].1 = None;
                        }
                        index
                    }
                    None => {
                        out.push((key, Some(field.name), Vec::new()));
                        out.len() - 1
                    }
                };
                if !field.selection_set.items.is_empty() {
                    out[index].2.push(&field.selection_set);
                }
            }
            gql::Selection::InlineFragment(fragment) => {
                collect_fields(fragments, &fragment.selection_set, out)?
            }
            gql::Selection::FragmentSpread(spread) => {
                let fragment = fragments
                    .iter()
                    .find(|f| f.name == spread.fragment_name)
                    .ok_or(Unsupported("unknown fragment"))?;
                collect_fields(fragments, &fragment.selection_set, out)?
            }
        }
    }
    Ok(())
}

/// The type condition every selection of `sets` is under, when they're all inline fragments on
/// the same type
pub(crate) fn single_type_condition<'a>(
    sets: &[&gql::SelectionSet<'a, &'a str>],
) -> Option<&'a str> {
    let mut found = None;
    for item in sets.iter().flat_map(|set| &set.items) {
        let gql::Selection::InlineFragment(fragment) = item else {
            return None;
        };
        let gql::TypeCondition::On(name) = fragment.type_condition.as_ref()?;
        if found.is_some_and(|found| found != *name) {
            return None;
        }
        found = Some(*name);
    }
    found
}

struct ShapeBuilder<'d, 'a> {
    fragments: &'d [&'d gql::FragmentDefinition<'a, &'a str>],
    renames: &'d [Rename],
    /// The keys from `data` to the object being built
    trail: Vec<&'a str>,
    shape: FetchShape,
}

impl<'d, 'a> ShapeBuilder<'d, 'a> {
    /// Adds the object the sets select at `position`. Returns its index. Fields of all type
    /// conditions are merged, like `ResponseShape` does for an object of an unknown type.
    fn object(
        &mut self,
        positions: &mut Positions,
        sets: &[&'d gql::SelectionSet<'a, &'a str>],
        position: u32,
        paths: Option<&CustomScalarPaths>,
    ) -> Result<u32, Unsupported> {
        let index = self.shape.objects.len() as u32;
        self.shape.objects.push(ShapeObject {
            layout: 0,
            nodes: (0, 0),
            position,
        });
        let mut fields: Vec<Collected<'d, 'a>> = Vec::new();
        for set in sets {
            self.collect(set, &mut fields)?;
        }
        let object_type = single_type_condition(sets);
        let type_range = self.text_range(object_type.unwrap_or(""));
        self.shape.object_types.push(type_range);
        let start = self.shape.nodes.len() as u32;
        // an aliased field the response has under its own key: the key the subgraph sends is
        // matched, its value goes where the field's own key is
        let logical = |trail: &[&str], key: &'a str| -> Option<&'d str> {
            self.renames
                .iter()
                .find(|rename| {
                    &*rename.wire == key
                        && rename.parent.len() == trail.len()
                        && rename.parent.iter().zip(trail).all(|(a, b)| &**a == *b)
                })
                .map(|rename| &*rename.to)
        };
        for (offset, (key, name, _)) in fields.iter().enumerate() {
            let name = self.text_range(name.unwrap_or(""));
            self.shape.names.push(name);
            if let Some(renamed) = logical(&self.trail, key) {
                self.shape
                    .renamed
                    .insert(start + offset as u32, renamed.into());
            }
            positions.add_key(position, logical(&self.trail, key).unwrap_or(key));
            let quoted = format!("\"{key}\"");
            if quoted.len() > u16::MAX as usize {
                return Err(Unsupported("key too long"));
            }
            let key_start = self.shape.text.len() as u32;
            self.shape.text.extend_from_slice(quoted.as_bytes());
            let mut prefix = [0u8; 8];
            let head = quoted.len().min(8);
            prefix[..head].copy_from_slice(&quoted.as_bytes()[..head]);
            let mut key_colon = [0u8; 16];
            if quoted.len() < 16 {
                key_colon[..quoted.len()].copy_from_slice(quoted.as_bytes());
                key_colon[quoted.len()] = b':';
            }
            self.shape.nodes.push(ShapeNode {
                prefix: u64::from_le_bytes(prefix),
                key_colon: u128::from_le_bytes(key_colon),
                key_start,
                key_len: quoted.len() as u32,
                field_index: NO_FIELD,
                object: NO_OBJECT,
                verify: NO_OBJECT,
                opaque_json: paths
                    .and_then(|paths| paths.child(key))
                    .is_some_and(|paths| paths.terminal),
            });
        }
        let end = self.shape.nodes.len() as u32;
        self.shape.objects[index as usize].nodes = (start, end);
        for (offset, (key, _, sets)) in fields.iter().enumerate() {
            let node = start as usize + offset;
            // a leaf keeps whatever is under it as it came, custom scalars included
            if self.shape.nodes[node].opaque_json || sets.is_empty() {
                continue;
            }
            let child_position =
                positions.child(position, logical(&self.trail, key).unwrap_or(key));
            let child_paths = paths.and_then(|paths| paths.child(key));
            self.trail.push(key);
            let object = self.object(positions, sets, child_position, child_paths);
            self.trail.pop();
            self.shape.nodes[node].object = object?;
        }
        Ok(index)
    }

    /// Appends `text` to the shape's text, returns its range
    fn text_range(&mut self, text: &str) -> (u32, u32) {
        let start = self.shape.text.len() as u32;
        self.shape.text.extend_from_slice(text.as_bytes());
        (start, text.len() as u32)
    }

    fn collect(
        &self,
        set: &'d gql::SelectionSet<'a, &'a str>,
        out: &mut Vec<Collected<'d, 'a>>,
    ) -> Result<(), Unsupported> {
        collect_fields(self.fragments, set, out)
    }
}
