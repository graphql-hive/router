//! A JSON parser for subgraph responses. It knows which fields the fetch asked for.
//!
//! # Why
//!
//! A subgraph answers the exact query that we send it. It sends the fields in the same order as
//! the query. sonic does not know this. It reads each key as a string, and it grows each object,
//! because it does not know the size. We know these things. So when we build the plan, we turn
//! the query of each fetch into a [`ResponseShape`]. Then we read each response of that fetch
//! with the shape:
//!
//! - We compare a key with the field that we expect next. This is one 8 byte compare. We do not
//!   read the key as a string.
//! - We allocate each object one time, with space for the fields of its type.
//! - The shape marks the custom scalars (we keep them as raw JSON). So we do not look up
//!   `CustomScalarPaths` while we read.
//!
//! The `Value` tree is the same, byte for byte.
//!
//! # What a shape looks like
//!
//! Here is a fetch:
//!
//! ```graphql
//! { me { id name } search { __typename ... on User { name } ... on Post { title } } }
//! ```
//!
//! [`ResponseShape::new`] makes this:
//!
//! ```text
//! objects  0 data    fields: nodes 0..2
//!          1 me      fields: nodes 2..4
//!          2 search  fields: nodes 4..7   types: 0..2
//!
//! nodes    0 "me" -> 1          1 "search" -> 2
//!          2 "id"               3 "name"
//!          4 "__typename" (T)   5 "name"    6 "title"    search, with the fields of all types
//!          7 "__typename" (T)   8 "title"                search when it is a Post
//!          9 "__typename" (T)  10 "name"                 search when it is a User
//!
//! types    0 "Post": nodes 7..9    1 "User": nodes 9..11    (sorted by name)
//! ```
//!
//! - An object is a run of nodes: its fields, in query order. `data` is always object 0.
//! - A node is one field. `-> 1` is the object in its value. `(T)` marks `__typename`.
//! - `search` can be a User or a Post. They select different fields, so `search` also has
//!   `types`. When the response has `"__typename":"User"`, the parser reads the rest of the
//!   object with nodes 9..11. It allocates space for 2 fields, not 3.
//! - All keys and type names are in one byte buffer, `text`. Nodes point into it.
//!
//! There are no pointers and no hash maps. There are only a few flat arrays, and indices into
//! them. So the shape has only a few allocations, and the main loop reads nodes that are next to
//! each other in memory.
//!
//! # How the parser reads a response
//!
//! [`parse_response`] reads the top-level object. It reads `data` with the shape. It gives
//! `errors` to sonic, because errors are rare and `GraphQLError` has its own serde rules. It
//! reads `extensions` as normal JSON. When the custom scalar paths make all of `data` a custom
//! scalar, it keeps `data` as raw JSON instead.
//!
//! `Parser::shaped_object` is the main loop. It runs for each key, so it must be fast. This is
//! how it reads `{"__typename":"User","name":"Kamil"}` as `search`:
//!
//! 1. It expects the fields of `search`. So the next field is node 4, `__typename`.
//! 2. It loads the next 8 bytes of the input, `"__typen`, as one `u64`. It compares this number
//!    with the prefixes of nodes 4 to 6 (`LOOKAHEAD` nodes or fewer). Node 4 matches. Its key is
//!    longer than 8 bytes, so it also compares the remaining bytes, `ame"`.
//! 3. The value is `"User"`. This is the first key, and it is `__typename`. So the object now
//!    uses the fields of User, nodes 9..11. The next field is node 10, after `__typename`. The
//!    object gets space for exactly 2 entries.
//! 4. The next 8 bytes are `"name":"`. The key of node 10, `"name"`, has 6 bytes. So it compares
//!    only the first 6 bytes, and they match.
//! 5. Then it finds `}`, and the object is done. There was one allocation. It did not read a key
//!    as a string, and it did not look anything up.
//!
//! Sometimes none of the next nodes match. For example, the fields are in a different order, the
//! field is of a different type, or we did not ask for the field.
//! Then `Parser::unexpected_key` looks up the key.
//!
//! Some values are not objects of the shape: scalars, `extensions`, and fields that we did not
//! ask for. `Parser::value` reads them. It is a normal JSON parser, and it makes the same `Value`s
//! as the sonic visitor.

use ahash::{HashMap, HashSet};
use smallvec::SmallVec;
use std::ops::Range;

use crate::executor::response::{
    graphql_error::GraphQLError,
    value::{bytes_eq, Value},
};
use crate::query_planner::ast::{
    document::Document, selection_item::SelectionItem, selection_set::SelectionSet,
};
use crate::query_planner::planner::plan_nodes::CustomScalarPaths;

/// Describes the expected fields and object types in a subgraph response.
///
/// Built once per fetch, alongside the query plan. Uses flat arrays and indices
/// instead of pointers to keep memory usage low and lookups fast.
#[derive(Debug, Clone)]
pub struct ResponseShape {
    /// Fields of each object, stored in query order. `data` is object 0.
    nodes: Box<[Node]>,
    /// Object definitions, referenced by index from `nodes`.
    objects: Box<[Object]>,
    /// Type-specific fields, sorted by type name.
    types: Box<[TypeFields]>,
    /// All field names (with quotes) and type names in a single byte buffer.
    text: Box<[u8]>,
    /// Whether the entire `data` value should be kept as raw JSON.
    raw_data: bool,
    /// Whether the planner left a `__typename` out of the query, so `fill_typenames` has
    /// something to do
    fills: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Span {
    start: u32,
    end: u32,
}

impl Span {
    #[inline]
    fn range(self) -> Range<usize> {
        self.start as usize..self.end as usize
    }

    #[inline]
    fn len(self) -> usize {
        (self.end - self.start) as usize
    }
}

/// A reference to a string in `text`, packed into a `u32`.
///
/// The lower 24 bits store the offset, and the upper 8 bits store the length.
/// Supports up to 16 MB of text and strings up to 255 bytes.
/// If these limits are exceeded, `ResponseShape::new` returns `None`.
#[derive(Debug, Clone, Copy)]
struct Text(u32);

const TEXT_LEN_SHIFT: u32 = 24;
const MAX_TEXT_OFFSET: usize = (1 << TEXT_LEN_SHIFT) - 1;

impl Text {
    #[inline]
    fn len(self) -> usize {
        (self.0 >> TEXT_LEN_SHIFT) as usize
    }

    #[inline]
    fn range(self) -> Range<usize> {
        let start = (self.0 & MAX_TEXT_OFFSET as u32) as usize;
        start..start + self.len()
    }
}

/// Number of upcoming fields to check before looking up a key.
///
/// Handles fields skipped by `@skip` or belonging to other types.
/// Limits comparisons to avoid slow lookups when fields arrive out of order.
///
/// Example: for `{ a b @skip(if: true) c @skip(if: true) d }` the response is
/// `{"a":..,"d":..}`. After `a`, we compare `b`, `c` and `d`. We find `d` without a lookup.
const LOOKAHEAD: usize = 4;

// The upper 2 bits of `Node.object` store flags, and the remaining bits store
// the object index. This avoids storing the flags separately.

/// Marks a custom scalar whose value is kept as raw JSON.
const RAW: u32 = 1 << 31;
/// Marks a `__typename` field.
const TYPENAME: u32 = 1 << 30;
/// Mask for extracting the object index.
const OBJECT_MASK: u32 = TYPENAME - 1;

/// A field in the response shape. Each node is 16 bytes, so four fit in a CPU cache line.
#[derive(Debug, Clone, Copy)]
struct Node {
    /// First 8 bytes of the quoted key
    prefix: u64,
    /// The quoted key, for example `"name"`. We compare it with the input as it is.
    key: Text,
    /// Object index, with `RAW` and `TYPENAME` flags in the upper bits.
    /// Index 0 means no nested object.
    object: u32,
}

impl Node {
    #[inline]
    fn object(self) -> u32 {
        self.object & OBJECT_MASK
    }

    #[inline]
    fn raw(self) -> bool {
        self.object & RAW != 0
    }

    #[inline]
    fn typename(self) -> bool {
        self.object & TYPENAME != 0
    }
}

/// Fields selected for an object type. Multiple types can share the same fields.
#[derive(Debug, Clone, Copy)]
struct TypeFields {
    name: Text,
    fields: Span,
}

#[derive(Debug, Clone, Copy, Default)]
struct Object {
    /// Fields from all type conditions, in query order.
    fields: Span,
    /// Type-specific fields that differ from `fields`.
    types: Span,
    /// Fields from custom scalar paths that are not in `fields`.
    /// Used to preserve raw JSON for unexpected fields.
    extra: Span,
    /// The object's type, when its field returns an object type and the query asks for its
    /// `__typename`. The planner leaves that `__typename` out of the query, so it's put back
    /// from here.
    typename: Option<Text>,
}

/// The first 8 bytes, with zeros at the end when there are fewer. Keys never have a zero byte,
/// so the zeros never match a key.
#[inline(always)]
fn first_bytes(bytes: &[u8]) -> [u8; 8] {
    let mut first = [0; 8];
    let head = bytes.len().min(8);
    first[..head].copy_from_slice(&bytes[..head]);
    first
}

/// The next 8 bytes of the input as one number. We compare keys with it.
#[inline(always)]
fn word(input: &[u8], pos: usize) -> u64 {
    match input.get(pos..pos + 8) {
        Some(bytes) => u64::from_le_bytes(bytes.try_into().expect("8 bytes")),
        None => u64::from_le_bytes(first_bytes(input.get(pos..).unwrap_or_default())),
    }
}

const ONES: u64 = 0x0101_0101_0101_0101;
/// The high bit of each byte. A byte with it is not ASCII.
const HIGH: u64 = 0x8080_8080_8080_8080;

/// The index of the first byte of the chunk that ends or breaks a string: a quote, a backslash or
/// a control character. 8 when there is none.
///
/// This is the usual trick to find a byte without a loop: after subtracting 1 from each byte, a
/// byte that was zero has its high bit set. Bytes above a match can be wrong, but the lowest
/// match is always right, and that is the one we take.
#[inline(always)]
fn special(chunk: u64) -> usize {
    let zero = |bytes: u64| bytes.wrapping_sub(ONES) & !bytes & HIGH;
    let quote = zero(chunk ^ (ONES * b'"' as u64));
    let backslash = zero(chunk ^ (ONES * b'\\' as u64));
    let control = chunk.wrapping_sub(ONES * 0x20) & !chunk & HIGH;
    ((quote | backslash | control).trailing_zeros() / 8) as usize
}

impl ResponseShape {
    /// Builds the shape of a fetch.
    ///
    /// `possible_types` returns the object types of an interface or a union. With it, an object
    /// knows which type conditions apply to it. `custom_scalar_paths` are the paths of the fetch,
    /// the same paths that sonic uses for its responses.
    ///
    /// Returns `None` when the shape does not fit in its indices: a key longer than 253 bytes, or
    /// more than 16 MB of keys.
    pub fn new<'s>(
        document: &Document,
        possible_types: impl Fn(&str) -> Option<&'s HashSet<String>>,
        custom_scalar_paths: Option<&CustomScalarPaths>,
    ) -> Option<Self> {
        Self::with_static_typenames(
            document,
            None,
            possible_types,
            |_, _| None,
            custom_scalar_paths,
        )
        .map(|(shape, _)| shape)
    }

    /// Same as `new`, but `field_type` gives the type a field returns, starting from the `root`
    /// type, so an object of an object type knows its `__typename`. Also says which selection
    /// sets of `document` can leave their `__typename` out, because every object they're
    /// selected on has it in `Object::typename`.
    pub fn with_static_typenames<'s>(
        document: &Document,
        root: Option<&'s str>,
        possible_types: impl Fn(&str) -> Option<&'s HashSet<String>>,
        field_type: impl Fn(&str, &str) -> Option<&'s str>,
        custom_scalar_paths: Option<&CustomScalarPaths>,
    ) -> Option<(Self, StaticTypenames)> {
        let paths = custom_scalar_paths.filter(|paths| !paths.is_empty());
        // In a request with context aliases, the keys have group numbers.
        // The shape does not have them.
        if paths.is_some_and(|paths| paths.group_suffixed) {
            return None;
        }
        let mut builder = Builder {
            document,
            possible_types,
            field_type,
            nodes: Vec::new(),
            objects: Vec::new(),
            types: Vec::new(),
            text: Vec::new(),
            fits: true,
            sets: HashMap::default(),
        };
        let raw_data = paths.is_some_and(|paths| paths.terminal);
        // the subgraph names its root type itself
        builder.object(&[&document.operation.selection_set], paths, root, false);
        let fills = !raw_data && builder.sets.values().any(|&typed| typed);
        builder.fits.then(|| {
            let shape = Self {
                nodes: builder.nodes.into(),
                objects: builder.objects.into(),
                types: builder.types.into(),
                text: builder.text.into(),
                raw_data,
                fills,
            };
            (shape, StaticTypenames(builder.sets))
        })
    }

    /// Puts back the `__typename` of every object the query asks for it on and the planner left
    /// it out of the query for, see `Object::typename`.
    pub fn fill_typenames<'a>(&'a self, data: &mut Value<'a>) {
        if self.fills {
            self.fill(data, 0);
        }
    }

    fn fill<'a>(&'a self, value: &mut Value<'a>, object: u32) {
        match value {
            Value::Array(items) => {
                for item in items {
                    self.fill(item, object);
                }
            }
            Value::Object(entries) => {
                let object = self.objects[object as usize];
                let typename = entries
                    .iter()
                    .find(|(key, _)| *key == "__typename")
                    .and_then(|(_, value)| value.as_str());
                let fields = match (typename, object.typename) {
                    (Some(name), _) => self
                        .type_fields(object.types, name.as_bytes())
                        .unwrap_or(object.fields),
                    (None, Some(name)) => {
                        // type names come from the schema, so they're valid UTF-8
                        let name = std::str::from_utf8(self.text(name)).expect("a type name");
                        entries.insert(0, ("__typename", Value::String(name.into())));
                        object.fields
                    }
                    (None, None) => object.fields,
                };
                for (key, value) in entries {
                    let node = self.nodes[fields.range()].iter().find(|node| {
                        let quoted = self.text(node.key);
                        &quoted[1..quoted.len() - 1] == key.as_bytes()
                    });
                    if let Some(node) = node.filter(|node| node.object() != 0) {
                        self.fill(value, node.object());
                    }
                }
            }
            _ => {}
        }
    }

    #[inline]
    fn text(&self, text: Text) -> &[u8] {
        &self.text[text.range()]
    }

    /// Returns true if the input at `pos` starts with the key of the node. `word` is the first 8
    /// bytes at `pos`.
    ///
    /// The key has its quotes. Because of this, a short compare is enough. `"id"` has 4 bytes, so
    /// we keep only the first 4 bytes of the word. These are the low bytes, because the word is
    /// little endian:
    ///
    /// ```text
    /// "id":1,"n   ->  "id"  matches
    /// "idx":1,"   ->  "idx  does not match, the closing quote is missing
    /// ```
    #[inline(always)]
    fn starts_with_key(&self, input: &[u8], pos: usize, word: u64, node: Node) -> bool {
        let len = node.key.len();
        if len <= 8 {
            // A quoted key has at least 2 bytes, so the shift is 48 or less.
            return word & (u64::MAX >> (64 - 8 * len)) == node.prefix;
        }
        word == node.prefix
            && input
                .get(pos + 8..pos + len)
                .is_some_and(|rest| bytes_eq(rest, &self.text(node.key)[8..]))
    }

    /// Returns the node in `fields` whose key is at `pos` in the input. `word` is the first 8
    /// bytes at `pos`.
    ///
    /// This compares the input with each node, without reading the key. For the few dozen
    /// fields an object usually has, that is faster than a lookup.
    #[inline]
    fn find(&self, fields: Span, input: &[u8], pos: usize, word: u64) -> Option<u32> {
        self.nodes[fields.range()]
            .iter()
            .position(|&node| self.starts_with_key(input, pos, word, node))
            .map(|index| fields.start + index as u32)
    }

    /// Returns the fields of an object of `types` when its `__typename` is `name`
    #[inline]
    fn type_fields(&self, types: Span, name: &[u8]) -> Option<Span> {
        let types = &self.types[types.range()];
        types
            .binary_search_by(|other| self.text(other.name).cmp(name))
            .ok()
            .map(|index| types[index].fields)
    }
}

/// A field of an object, and the selection sets of each place that selects it.
///
/// In `{ u { ... on A { n { x } } ... on B { n { y } } } }`, the fields of `u` have one `n` with
/// two sets, `{ x }` and `{ y }`. So the object in `n` has `x` and `y`.
struct Field<'d, 's> {
    key: &'d str,
    sets: SmallVec<[&'d SelectionSet; 2]>,
    /// The type it returns, `None` when we don't know it or its places don't agree
    ty: Option<&'s str>,
}

impl Field<'_, '_> {
    /// Returns true if the two lists have the same keys from the same selection sets. Then they
    /// make the same nodes. All sets come from one document, so we can compare pointers.
    fn same(a: &[Field], b: &[Field]) -> bool {
        a.len() == b.len()
            && a.iter().zip(b).all(|(a, b)| {
                a.key == b.key
                    && a.sets.len() == b.sets.len()
                    && a.sets
                        .iter()
                        .zip(&b.sets)
                        .all(|(a, b)| std::ptr::eq(*a, *b))
            })
    }
}

/// The selection sets whose `__typename` execution fills in itself.
pub struct StaticTypenames(HashMap<*const SelectionSet, bool>);

impl StaticTypenames {
    pub fn contains(&self, set: &SelectionSet) -> bool {
        self.0.get(&(set as *const _)) == Some(&true)
    }
}

struct Builder<'d, F, T> {
    document: &'d Document,
    possible_types: F,
    field_type: T,
    nodes: Vec<Node>,
    objects: Vec<Object>,
    types: Vec<TypeFields>,
    text: Vec<u8>,
    /// false when something does not fit in its bits
    fits: bool,
    /// Whether every object a selection set is selected on has `Object::typename`
    sets: HashMap<*const SelectionSet, bool>,
}

/// The type the fields of a fragment are looked up on. A fragment without a type condition
/// keeps the type it's in.
fn condition_or<'a>(condition: &'a str, parent: Option<&'a str>) -> Option<&'a str> {
    if condition.is_empty() {
        parent
    } else {
        Some(condition)
    }
}

/// The run of `vec` from `start` to its end
fn span_from<T>(start: usize, vec: &[T]) -> Span {
    Span {
        start: start as u32,
        end: vec.len() as u32,
    }
}

impl<'d, 's, F, T> Builder<'d, F, T>
where
    F: Fn(&str) -> Option<&'s HashSet<String>>,
    T: Fn(&str, &str) -> Option<&'s str>,
{
    /// Adds the object that the sets select, with the custom scalar paths under it. Returns its
    /// index.
    ///
    /// It goes depth first. First it adds the fields of the object as one run of nodes. Then it
    /// adds the objects in those fields (and their objects). Then it adds a run for the keys that
    /// only the custom scalar paths have (see `Object::extra`). Then it adds one run for each type
    /// that selects different fields. In the module docs, `search` is nodes 4..7, and the fields
    /// of User and Post come after them.
    ///
    /// Example: User, Post and Tag implement `Node`, and the query is
    /// `{ node { __typename id ... on User { name } ... on Post { name } ... on Tag { id } } }`.
    /// The types of `node` are:
    ///
    /// ```text
    /// all     __typename id name
    /// Post    __typename id name   the same as all, so not added, the parser uses all
    /// Tag     __typename id        its own run
    /// User    __typename id name   the same as all, so not added
    /// ```
    ///
    /// If two types select the same fields, and those are not all the fields, they share one run.
    ///
    /// `ty` is the type the field of the object returns. `typed` is false when the object can
    /// have another `__typename` than `ty`.
    fn object(
        &mut self,
        sets: &[&'d SelectionSet],
        paths: Option<&'d CustomScalarPaths>,
        ty: Option<&'s str>,
        typed: bool,
    ) -> u32 {
        // Get the index first, so `data` is 0.
        let index = self.objects.len();
        self.objects.push(Object::default());

        let merged = self.fields(sets, ty, None);
        let fields = self.flatten(&merged, paths);
        // an object type is the only possible type of itself
        let typename = ty
            .filter(|ty| {
                typed
                    && (self.possible_types)(ty)
                        .is_some_and(|types| types.len() == 1 && types.contains(*ty))
            })
            .filter(|_| merged.iter().any(|field| field.key == "__typename"));
        for set in sets {
            self.record(set, typename.is_some());
        }
        let typename = typename.map(|ty| self.text(ty.as_bytes()));
        // Keys of the custom scalar paths that no type selects. See `Object::extra`.
        let extra: Vec<Field> = paths
            .into_iter()
            .flat_map(|paths| &paths.children)
            .filter(|(key, paths)| !paths.is_empty() && !merged.iter().any(|f| f.key == *key))
            .map(|(key, _)| Field {
                key,
                sets: SmallVec::new(),
                ty: None,
            })
            .collect();
        let extra = self.flatten(&extra, paths);

        let mut names = Vec::new();
        for set in sets {
            self.conditions(set, &mut names);
        }
        // Sort them, so the parser can use a binary search.
        names.sort_unstable();
        names.dedup();
        let mut lists: Vec<(Vec<Field>, Span)> = Vec::new();
        // The fields of a type add objects, and those add their types. So we add the types of
        // this object after that, in one run.
        let mut types = Vec::new();
        for name in names {
            let own = self.fields(sets, ty, Some(name));
            if Field::same(&own, &merged) {
                continue;
            }
            let fields = match lists.iter().find(|(list, _)| Field::same(list, &own)) {
                Some((_, fields)) => *fields,
                None => {
                    let fields = self.flatten(&own, paths);
                    lists.push((own, fields));
                    fields
                }
            };
            types.push((name, fields));
        }
        let start = self.types.len();
        for (name, fields) in types {
            let name = self.text(name.as_bytes());
            self.types.push(TypeFields { name, fields });
        }
        self.objects[index] = Object {
            fields,
            types: span_from(start, &self.types),
            extra,
            typename,
        };
        self.fits &= index <= OBJECT_MASK as usize;
        index as u32
    }

    fn record(&mut self, set: &'d SelectionSet, typed: bool) {
        let entry = self.sets.entry(set as *const _).or_insert(typed);
        *entry &= typed;
        for item in &set.items {
            match item {
                SelectionItem::Field(_) => {}
                SelectionItem::InlineFragment(fragment) => self.record(&fragment.selections, typed),
                SelectionItem::FragmentSpread(name) => {
                    if let Some(fragment) = self.document.fragments.iter().find(|f| &f.name == name)
                    {
                        self.record(&fragment.selection_set, typed);
                    }
                }
            }
        }
    }

    fn text(&mut self, bytes: &[u8]) -> Text {
        let start = self.text.len();
        self.text.extend_from_slice(bytes);
        self.fits &= start <= MAX_TEXT_OFFSET && bytes.len() <= u8::MAX as usize;
        Text(start as u32 & MAX_TEXT_OFFSET as u32 | (bytes.len() as u32 & 0xff) << TEXT_LEN_SHIFT)
    }

    /// Returns the object types that its type conditions match. An interface or a union becomes
    /// its object types.
    ///
    /// `... on Node { ... on User { .. } }` gives each type that implements `Node`, and `User`.
    /// Only these types can have different fields than all the fields. So only these types get
    /// their own list.
    fn conditions<'x>(&self, set: &'d SelectionSet, out: &mut Vec<&'x str>)
    where
        'd: 'x,
        's: 'x,
    {
        for item in &set.items {
            let (condition, set) = match item {
                SelectionItem::Field(_) => continue,
                SelectionItem::InlineFragment(fragment) => {
                    (fragment.type_condition.as_str(), &fragment.selections)
                }
                SelectionItem::FragmentSpread(name) => {
                    match self.document.fragments.iter().find(|f| &f.name == name) {
                        Some(fragment) => {
                            (fragment.type_condition.as_str(), &fragment.selection_set)
                        }
                        None => continue,
                    }
                }
            };
            match (self.possible_types)(condition) {
                Some(types) => out.extend(types.iter().map(String::as_str)),
                None if !condition.is_empty() => out.push(condition),
                None => {}
            }
            self.conditions(set, out);
        }
    }

    fn applies(&self, condition: &str, typename: Option<&str>) -> bool {
        match typename {
            None => true,
            Some(typename) => {
                condition.is_empty()
                    || condition == typename
                    || (self.possible_types)(condition)
                        .is_some_and(|types| types.contains(typename))
            }
        }
    }

    /// Returns the fields of an object of `typename`. When `typename` is `None`, returns the
    /// fields of all type conditions.
    ///
    /// The fields are in the order that they first appear. This is the order that the subgraph
    /// sends them in. A field that is selected two times is one field. We use the response key,
    /// so each alias is its own field. `parent` is the type the fields are looked up on.
    fn fields(
        &self,
        sets: &[&'d SelectionSet],
        parent: Option<&str>,
        typename: Option<&str>,
    ) -> Vec<Field<'d, 's>> {
        let mut out = Vec::new();
        for set in sets {
            self.collect(set, parent, typename, &mut out);
        }
        out
    }

    fn collect(
        &self,
        set: &'d SelectionSet,
        parent: Option<&str>,
        typename: Option<&str>,
        out: &mut Vec<Field<'d, 's>>,
    ) {
        for item in &set.items {
            match item {
                SelectionItem::Field(field) => {
                    let key = field.selection_identifier();
                    let ty = parent.and_then(|parent| (self.field_type)(parent, &field.name));
                    let index = match out.iter().position(|f| f.key == key) {
                        Some(index) => {
                            if out[index].ty != ty {
                                out[index].ty = None;
                            }
                            index
                        }
                        None => {
                            out.push(Field {
                                key,
                                sets: SmallVec::new(),
                                ty,
                            });
                            out.len() - 1
                        }
                    };
                    if !field.selections.items.is_empty() {
                        out[index].sets.push(&field.selections);
                    }
                }
                SelectionItem::InlineFragment(fragment) => {
                    if self.applies(&fragment.type_condition, typename) {
                        let condition = condition_or(&fragment.type_condition, parent);
                        self.collect(&fragment.selections, condition, typename, out)
                    }
                }
                SelectionItem::FragmentSpread(name) => {
                    if let Some(fragment) = self.document.fragments.iter().find(|f| &f.name == name)
                    {
                        if self.applies(&fragment.type_condition, typename) {
                            let condition = condition_or(&fragment.type_condition, parent);
                            self.collect(&fragment.selection_set, condition, typename, out)
                        }
                    }
                }
            }
        }
    }

    /// Adds a run of nodes for the fields, and then the objects in their values.
    ///
    /// We add all nodes of the run before any child object. So the fields of an object are next
    /// to each other in `nodes`, and its child objects come later. We know the index of a child
    /// only after we add it, so we put it into `object` at the end.
    fn flatten(&mut self, fields: &[Field<'d, 's>], paths: Option<&'d CustomScalarPaths>) -> Span {
        let start = self.nodes.len();
        for field in fields {
            let quoted = format!("\"{}\"", field.key);
            let raw = paths
                .and_then(|paths| paths.child(field.key))
                .is_some_and(|paths| paths.terminal);
            let mut flags = 0;
            if raw {
                flags |= RAW;
            }
            if field.key == "__typename" {
                flags |= TYPENAME;
            }
            let key = self.text(quoted.as_bytes());
            self.nodes.push(Node {
                prefix: u64::from_le_bytes(first_bytes(quoted.as_bytes())),
                key,
                object: flags,
            });
        }
        let nodes = span_from(start, &self.nodes);
        for (index, field) in nodes.range().zip(fields) {
            // A custom scalar stays as it is, also when it is an object. So we do not read
            // anything under it.
            if self.nodes[index].raw() {
                continue;
            }
            // Add an object for the selected fields. Also add one for custom scalar paths under
            // a field that selects nothing for this type (see `Object::extra`).
            let child = paths
                .and_then(|paths| paths.child(field.key))
                .filter(|paths| !paths.is_empty());
            if !field.sets.is_empty() || child.is_some() {
                let object = self.object(&field.sets, child, field.ty, true);
                self.nodes[index].object |= object & OBJECT_MASK;
            }
        }
        nodes
    }
}

pub struct ParsedResponse<'a> {
    pub data: Value<'a>,
    pub errors: Option<Vec<GraphQLError>>,
    pub extensions: Option<Value<'a>>,
}

/// The top-level keys that we read, as bits
const DATA: u8 = 1;
const ERRORS: u8 = 1 << 1;
const EXTENSIONS: u8 = 1 << 2;

/// Sets `bit`, and returns true if it was not set before.
#[inline]
fn first(seen: &mut u8, bit: u8) -> bool {
    let first = *seen & bit == 0;
    *seen |= bit;
    first
}

/// Reads a response with the shape. Returns `None` when sonic must parse the response. The
/// custom scalars are the ones from when we built the shape.
pub fn parse_response<'a>(input: &'a [u8], shape: &ResponseShape) -> Option<ParsedResponse<'a>> {
    let mut parser = Parser {
        input,
        pos: 0,
        depth: 0,
        shape,
        elements: Vec::new(),
        entries: Vec::new(),
    };
    let mut response = ParsedResponse {
        data: Value::Null,
        errors: None,
        extensions: None,
    };
    let mut seen = 0;

    parser.whitespace();
    if parser.peek() != Some(b'{') {
        return None;
    }
    parser.pos += 1;
    if !parser.empty(b'}') {
        loop {
            parser.whitespace();
            let key = parser.key()?;
            parser.expect(b':')?;
            match key {
                // A second one is an error. sonic writes the error.
                "data" if first(&mut seen, DATA) => {
                    response.data = if shape.raw_data {
                        parser.raw()?
                    } else {
                        parser.shaped(0)?
                    };
                }
                // Errors are rare, and `GraphQLError` has its own serde rules (optional fields,
                // path segments, extensions). So sonic reads this part, and gives the same
                // errors.
                "errors" if first(&mut seen, ERRORS) => {
                    parser.whitespace();
                    let start = parser.pos;
                    parser.skip_value()?;
                    let errors = &input[start..parser.pos];
                    response.errors = Some(sonic_rs::from_slice(errors).ok()?);
                }
                "extensions" if first(&mut seen, EXTENSIONS) => {
                    response.extensions = Some(parser.value()?);
                }
                "data" | "errors" | "extensions" => return None,
                _ => parser.skip_value()?,
            }
            if !parser.more(b'}')? {
                break;
            }
        }
    }
    parser.whitespace();
    (parser.pos == input.len()).then_some(response)
}

/// sonic allows 254 levels. If a response goes deeper than this, sonic reads it. So we never run
/// out of stack.
///
/// The parser calls itself one time for each object or list. So a body with 10 000 `[` would use
/// all of the stack. The parser does not know the depth before it starts. It stops when it goes
/// past the limit, and then sonic parses the body again. Real GraphQL responses have only a few
/// levels, so only strange or hostile bodies pay for this.
const MAX_DEPTH: u32 = 128;

struct Parser<'a, 's> {
    input: &'a [u8],
    pos: usize,
    depth: u32,
    shape: &'s ResponseShape,
    // The children of the objects and lists that we are reading. With this, each object or list
    // gets one allocation with the exact size, like DeserializeScratch. For `[1,[2,3]]` the stack
    // is `1`, then `1 2 3`. Then `[2,3]` becomes its own `Vec` with 2 values. Then `1 [2,3]`
    // becomes the outer list. Shaped objects do not use this, because they know their size.
    elements: Vec<Value<'a>>,
    entries: Vec<(&'a str, Value<'a>)>,
}

impl<'a> Parser<'a, '_> {
    #[inline]
    fn peek(&self) -> Option<u8> {
        self.input.get(self.pos).copied()
    }

    #[inline]
    fn whitespace(&mut self) {
        while let Some(b' ' | b'\n' | b'\r' | b'\t') = self.peek() {
            self.pos += 1;
        }
    }

    #[inline]
    fn expect(&mut self, byte: u8) -> Option<()> {
        // Usually there is no whitespace, so we look at the byte first.
        if self.peek() != Some(byte) {
            self.whitespace();
            if self.peek() != Some(byte) {
                return None;
            }
        }
        self.pos += 1;
        Some(())
    }

    /// Call it after a value in an object or a list. Returns true after a comma, and false at
    /// the end.
    #[inline]
    fn more(&mut self, close: u8) -> Option<bool> {
        let mut byte = self.peek()?;
        if byte != b',' && byte != close {
            self.whitespace();
            byte = self.peek()?;
        }
        self.pos += 1;
        match byte {
            b',' => Some(true),
            _ if byte == close => Some(false),
            _ => None,
        }
    }

    /// Skips `close`, and returns true if the object or list is empty.
    #[inline]
    fn empty(&mut self, close: u8) -> bool {
        self.whitespace();
        let empty = self.peek() == Some(close);
        self.pos += empty as usize;
        empty
    }

    #[inline]
    fn enter(&mut self) -> Option<()> {
        self.depth += 1;
        (self.depth <= MAX_DEPTH).then_some(())
    }

    fn value(&mut self) -> Option<Value<'a>> {
        self.whitespace();
        match self.peek()? {
            b'"' => self.string(),
            b'{' => self.object(),
            b'[' => self.list(None),
            b't' => self.literal(b"true", Value::Bool(true)),
            b'f' => self.literal(b"false", Value::Bool(false)),
            b'n' => self.literal(b"null", Value::Null),
            b'-' | b'0'..=b'9' => self.number(),
            _ => None,
        }
    }

    /// Reads a value that we expect to be an object of `object` in the shape, or a list of them.
    fn shaped(&mut self, object: u32) -> Option<Value<'a>> {
        self.whitespace();
        match self.peek()? {
            b'{' => self.shaped_object(object),
            b'[' => self.list(Some(object)),
            _ => self.value(),
        }
    }

    /// The main loop. See "How the parser reads a response" in the module docs for an example.
    ///
    /// For each key, the fast path is: load 8 bytes, compare them with up to `LOOKAHEAD`
    /// prefixes, and skip the length of the key. All other work is in `unexpected_key`. It is a
    /// separate function, so this loop stays small. When it was part of this loop, the parser
    /// was 6 to 9% slower.
    fn shaped_object(&mut self, object: u32) -> Option<Value<'a>> {
        self.pos += 1;
        if self.empty(b'}') {
            return Some(Value::Object(Vec::new()));
        }
        self.enter()?;
        let shape = self.shape;
        let object = shape.objects[object as usize];
        // The fields that we expect. At first, these are the fields of all types. When
        // `__typename` tells the type, these are the fields of that type.
        let mut fields = object.fields;
        // We allocate after the first entry, because it can tell the type of the object.
        let mut entries = Vec::new();
        // The node that we expect the next key to be
        let mut next = fields.start;
        loop {
            self.whitespace();
            // Usually this is the next field. It can be a later field, when the fields between
            // are not there (fields of another type, or @skip).
            let word = word(self.input, self.pos);
            let ahead = if next < fields.end
                && shape.starts_with_key(self.input, self.pos, word, shape.nodes[next as usize])
            {
                Some(0)
            } else {
                let lookahead =
                    next as usize + 1..(next as usize + LOOKAHEAD).min(fields.end as usize);
                shape.nodes.get(lookahead).and_then(|nodes| {
                    nodes
                        .iter()
                        .position(|&node| shape.starts_with_key(self.input, self.pos, word, node))
                        .map(|ahead| ahead + 1)
                })
            };
            let (key, node) = match ahead {
                Some(ahead) => {
                    let index = next as usize + ahead;
                    next = index as u32 + 1;
                    let node = shape.nodes[index];
                    (self.take_key(node), Some(node))
                }
                None => {
                    let (key, index) = self.unexpected_key(object, fields, word)?;
                    if let Some(index) =
                        index.filter(|&index| fields.range().contains(&(index as usize)))
                    {
                        // The fields after it probably come next.
                        next = index + 1;
                    }
                    (key, index.map(|index| shape.nodes[index as usize]))
                }
            };
            self.expect(b':')?;
            let value = match node {
                Some(node) if node.raw() => self.raw()?,
                Some(node) if node.object() != 0 => self.shaped(node.object())?,
                _ => self.value()?,
            };
            // Nothing is allocated before the first entry. So this means "after the first key",
            // and we do not need another variable.
            if entries.capacity() == 0 {
                // Planned queries ask for __typename first. So an object of an abstract type can
                // read the other fields with the fields of its own type. If __typename is not
                // first, is not a string, or is a type that we do not know, the object keeps the
                // fields of all types. Then the object is only a bit larger than necessary.
                let typename = node.map_or(key == "__typename", Node::typename);
                if let (true, Value::String(name)) = (typename, &value) {
                    if let Some(own) = shape.type_fields(object.types, name.as_bytes()) {
                        fields = own;
                        // Continue after the __typename of the type. The type has one, because
                        // it selected it.
                        next = shape.nodes[own.range()]
                            .iter()
                            .position(|node| node.typename())
                            .map_or(own.start, |index| own.start + index as u32 + 1);
                    }
                }
                entries.reserve_exact(fields.len());
            }
            entries.push((key, value));
            if !self.more(b'}')? {
                self.depth -= 1;
                return Some(Value::Object(entries));
            }
        }
    }

    /// Moves past the quoted key of the node, which is at the start of the input. Returns the
    /// key.
    #[inline]
    fn take_key(&mut self, node: Node) -> &'a str {
        let start = self.pos + 1;
        self.pos += node.key.len();
        // SAFETY: the bytes are the key of the node, which is a `str` of the query
        unsafe { std::str::from_utf8_unchecked(&self.input[start..self.pos - 1]) }
    }

    /// Reads a key that is not one of the next few fields. The key is in a different order, or
    /// of another type, or the shape does not know it. Returns the key and its node. The node is
    /// `None` when the shape does not know the key.
    ///
    /// Example: `search` from the module docs, read as a User (nodes 9..11):
    ///
    /// ```text
    /// "name"   in a different order, for example after a field that we did not ask for:
    ///          found in the fields of User
    /// "title"  a field of Post in a User: not in the fields of User, found in all the fields
    /// "extra"  nobody asked for it: no node, read as normal JSON and kept
    /// "meta"   only the custom scalar paths have it: found in `Object::extra`, kept as raw JSON
    /// ```
    #[cold]
    #[inline(never)]
    fn unexpected_key(
        &mut self,
        object: Object,
        fields: Span,
        word: u64,
    ) -> Option<(&'a str, Option<u32>)> {
        let shape = self.shape;
        let index = shape
            .find(fields, self.input, self.pos, word)
            // A field of another type than the type that the object told us
            .or_else(|| {
                (fields != object.fields)
                    .then(|| shape.find(object.fields, self.input, self.pos, word))
                    .flatten()
            })
            // A key that only the custom scalar paths have
            .or_else(|| shape.find(object.extra, self.input, self.pos, word));
        let key = match index {
            Some(index) => self.take_key(shape.nodes[index as usize]),
            None => self.key()?,
        };
        Some((key, index))
    }

    fn object(&mut self) -> Option<Value<'a>> {
        self.pos += 1;
        if self.empty(b'}') {
            return Some(Value::Object(Vec::new()));
        }
        self.enter()?;
        let start = self.entries.len();
        loop {
            self.whitespace();
            let key = self.key()?;
            self.expect(b':')?;
            let value = self.value()?;
            self.entries.push((key, value));
            if !self.more(b'}')? {
                break;
            }
        }
        self.depth -= 1;
        Some(Value::Object(self.entries.split_off(start)))
    }

    fn list(&mut self, object: Option<u32>) -> Option<Value<'a>> {
        self.pos += 1;
        if self.empty(b']') {
            return Some(Value::Array(Vec::new()));
        }
        self.enter()?;
        let start = self.elements.len();
        loop {
            let value = match object {
                Some(object) => self.shaped(object)?,
                None => self.value()?,
            };
            self.elements.push(value);
            if !self.more(b']')? {
                break;
            }
        }
        self.depth -= 1;
        Some(Value::Array(self.elements.split_off(start)))
    }

    /// Checks a value like `value` does, but does not build it.
    fn skip_value(&mut self) -> Option<()> {
        self.whitespace();
        let close = match self.peek()? {
            b'{' => b'}',
            b'[' => b']',
            _ => return self.value().map(drop),
        };
        self.pos += 1;
        if self.empty(close) {
            return Some(());
        }
        self.enter()?;
        loop {
            if close == b'}' {
                self.whitespace();
                self.key()?;
                self.expect(b':')?;
            }
            self.skip_value()?;
            if !self.more(close)? {
                break;
            }
        }
        self.depth -= 1;
        Some(())
    }

    /// Reads a custom scalar. We keep it as the JSON that it came as.
    fn raw(&mut self) -> Option<Value<'a>> {
        self.whitespace();
        let start = self.pos;
        self.skip_value()?;
        // SAFETY: `skip_value` checked each string, and all other bytes of a JSON value are ASCII
        let raw = unsafe { std::str::from_utf8_unchecked(&self.input[start..self.pos]) };
        Some(Value::RawJson(raw.into()))
    }

    /// Keys point into the input, so if a key has escapes, sonic must reject it.
    ///
    /// sonic does the same for `"a\u0062"`. It cannot point into the input for that key either,
    /// so it returns an error. When we stop here, the response gets that same error.
    fn key(&mut self) -> Option<&'a str> {
        if self.peek() != Some(b'"') {
            return None;
        }
        match self.raw_string()? {
            (key, false) => Some(key),
            (_, true) => None,
        }
    }

    fn string(&mut self) -> Option<Value<'a>> {
        let start = self.pos;
        let (raw, escaped) = self.raw_string()?;
        if !escaped {
            return Some(Value::String(raw.into()));
        }
        let decoded: String = sonic_rs::from_slice(&self.input[start..self.pos]).ok()?;
        Some(Value::String(decoded.into()))
    }

    /// Returns the bytes between the quotes, and true if they have escapes to decode.
    ///
    /// Most strings are short: ids, names, enum values. For these, one or two 8 byte steps find
    /// the end, and check for backslashes and control characters on the way. A longer string
    /// goes to memchr, which uses SIMD, and then one pass checks it for control characters.
    fn raw_string(&mut self) -> Option<(&'a str, bool)> {
        let start = self.pos + 1;
        let mut end = start;
        let mut escaped = false;
        // Whether the string can have bytes that are not ASCII. Then we check that it is UTF-8.
        let mut non_ascii = false;
        let mut steps = 2;
        loop {
            if steps > 0 {
                steps -= 1;
                let chunk = word(self.input, end);
                non_ascii |= chunk & HIGH != 0;
                let at = special(chunk);
                end += at;
                if at == 8 {
                    continue;
                }
            } else {
                let from = end;
                // A backslash at the very end moves past the end, so we use `get`.
                end += memchr::memchr2(b'"', b'\\', self.input.get(end..)?)?;
                // We check all bytes with `|` and do not stop early, so the compiler can use SIMD.
                let (mut control, mut high) = (false, false);
                for &byte in &self.input[from..end] {
                    control |= byte < 0x20;
                    high |= byte >= 0x80;
                }
                if control {
                    return None;
                }
                non_ascii |= high;
            }
            match *self.input.get(end)? {
                b'"' => break,
                b'\\' => {
                    escaped = true;
                    end += 2;
                }
                // JSON strings cannot have these bytes without an escape.
                _ => return None,
            }
        }
        let raw = &self.input[start..end];
        self.pos = end + 1;
        // The scan skips the byte after a backslash, so an escaped string gets the full check.
        let raw = if non_ascii || escaped {
            std::str::from_utf8(raw).ok()?
        } else {
            // SAFETY: all bytes are ASCII
            unsafe { std::str::from_utf8_unchecked(raw) }
        };
        Some((raw, escaped))
    }

    fn literal(&mut self, word: &[u8], value: Value<'a>) -> Option<Value<'a>> {
        if !self.input[self.pos..].starts_with(word) {
            return None;
        }
        self.pos += word.len();
        Some(value)
    }

    fn digits(&mut self) -> usize {
        let start = self.pos;
        while let Some(b'0'..=b'9') = self.peek() {
            self.pos += 1;
        }
        self.pos - start
    }

    fn number(&mut self) -> Option<Value<'a>> {
        let start = self.pos;
        let negative = self.peek() == Some(b'-');
        self.pos += negative as usize;
        let integer = self.pos;
        // We read the number while we scan. This is only correct for up to 19 digits.
        let mut magnitude = 0u64;
        // No leading zeros: `0` alone, or a number that starts with 1 to 9.
        match self.peek()? {
            b'0' => self.pos += 1,
            b'1'..=b'9' => {
                while let Some(digit @ b'0'..=b'9') = self.peek() {
                    magnitude = magnitude
                        .wrapping_mul(10)
                        .wrapping_add((digit - b'0') as u64);
                    self.pos += 1;
                }
            }
            _ => return None,
        }
        let integer = &self.input[integer..self.pos];
        let mut float = false;
        if self.peek() == Some(b'.') {
            self.pos += 1;
            if self.digits() == 0 {
                return None;
            }
            float = true;
        }
        if let Some(b'e' | b'E') = self.peek() {
            self.pos += 1;
            if let Some(b'+' | b'-') = self.peek() {
                self.pos += 1;
            }
            if self.digits() == 0 {
                return None;
            }
            float = true;
        }
        if !float {
            let magnitude = if integer.len() <= 19 {
                Some(magnitude)
            } else {
                integer.iter().try_fold(0u64, |n, &digit| {
                    n.checked_mul(10)?.checked_add((digit - b'0') as u64)
                })
            };
            match magnitude {
                Some(n) if !negative => return Some(Value::U64(n)),
                // sonic reads -0 as a float.
                Some(n) if n != 0 && n <= i64::MIN.unsigned_abs() => {
                    return Some(Value::I64((n as i64).wrapping_neg()))
                }
                _ => {}
            }
        }
        // core and sonic both round correctly, so the bits are the same. Two differences: sonic
        // reads -0 as 0, and it rejects a number that does not fit in an f64.
        // SAFETY: we read only ASCII digits, signs, a dot and an exponent
        let text = unsafe { std::str::from_utf8_unchecked(&self.input[start..self.pos]) };
        let float: f64 = text.parse().ok()?;
        float
            .is_finite()
            .then_some(Value::F64(if float == 0.0 { 0.0 } else { float }))
    }
}

#[cfg(test)]
mod tests {
    use ahash::{HashSet, HashSetExt};

    use bytes::Bytes;
    use graphql_tools::parser::query;

    use super::{parse_response, ResponseShape, MAX_DEPTH};
    use crate::executor::response::subgraph_response::SubgraphResponse;
    use crate::executor::response::value::Value;
    use crate::query_planner::ast::{
        document::Document, fragment::FragmentDefinition, operation::OperationDefinition,
        selection_item::SelectionItem, selection_set::SelectionSet,
    };
    use crate::query_planner::planner::plan_nodes::CustomScalarPaths;
    use crate::query_planner::utils::parsing::parse_operation;

    fn document(source: &str) -> Document {
        let mut selection_set = None;
        let mut fragments = Vec::new();
        for definition in parse_operation(source).definitions {
            match definition {
                query::Definition::Operation(query::OperationDefinition::SelectionSet(set)) => {
                    selection_set = Some(set.into())
                }
                query::Definition::Operation(query::OperationDefinition::Query(q)) => {
                    selection_set = Some(q.selection_set.into())
                }
                query::Definition::Fragment(fragment) => {
                    let query::TypeCondition::On(type_condition) = fragment.type_condition;
                    fragments.push(FragmentDefinition {
                        name: fragment.name,
                        selection_set: fragment.selection_set.into(),
                        type_condition,
                    });
                }
                query::Definition::Operation(_) => {}
            }
        }
        Document {
            operation: OperationDefinition {
                name: None,
                operation_kind: None,
                selection_set: selection_set.expect("a query"),
                variable_definitions: None,
            },
            fragments,
        }
    }

    fn shape(source: &str, paths: Option<&CustomScalarPaths>) -> ResponseShape {
        // `I` is an interface. A and C implement it.
        let mut interface = HashSet::new();
        interface.extend(["A".to_string(), "C".to_string()]);
        ResponseShape::new(
            &document(source),
            |name| (name == "I").then_some(&interface),
            paths,
        )
        .expect("a shape")
    }

    const QUERY: &str = "{ a b { c d } list { x y }
        u { __typename ... on A { a1 n { x } } ... on B { b1 n { y } } ...F ... on I { i } } }
        fragment F on A { f }";

    /// The shaped parser must give the same result as sonic for a body: the same response, or
    /// the same error.
    fn same_as_sonic(body: &[u8], paths: Option<&CustomScalarPaths>) {
        same_as_sonic_for(QUERY, body, paths)
    }

    fn same_as_sonic_for(query: &str, body: &[u8], paths: Option<&CustomScalarPaths>) {
        let shape = shape(query, paths);
        let bytes = Bytes::copy_from_slice(body);
        let shaped = SubgraphResponse::deserialize_with_shape(bytes.clone(), &shape, paths);
        let sonic = SubgraphResponse::deserialize_from_bytes(bytes, paths);
        assert_eq!(
            format!("{shaped:?}"),
            format!("{sonic:?}"),
            "{}",
            String::from_utf8_lossy(body)
        );
    }

    #[test]
    fn reads_what_sonic_reads() {
        let bodies: &[&str] = &[
            r#"{"data":{"a":1,"b":{"c":"x","d":null},"list":[{"x":1,"y":2},{"x":3,"y":4}]}}"#,
            // a different order, missing and extra fields, nulls and nested lists
            r#"{"data":{"list":[{"y":3,"x":4,"z":{"k":[1]}},null,[{"x":5}]],"b":{"d":[]},"a":{"x":1}}}"#,
            // a field of one type, then of another type, then of a fragment
            r#"{"data":{"u":[{"__typename":"B","b1":1},{"__typename":"A","a1":2,"f":3},{}]}}"#,
            // a field before a field that is earlier in the shape
            r#"{"data":{"u":[{"b1":1,"__typename":"B","x":2,"a1":3}]}}"#,
            r#"{"data":null,"errors":[{"message":"boom","path":["a",0],"extensions":{"code":"X"}}]}"#,
            r#"{"errors":[{"message":"only errors"}]}"#,
            r#"{"data":{},"extensions":{"x":[1,{"y":true}],"n":null}}"#,
            r#"{"hasNext":false,"data":{"a":1},"other":{"deep":[[[]]]}}"#,
            r#"{"data":{"a":{},"b":[],"list":[]}}"#,
            " \n\t{ \"data\" : { \"a\" : [ 1 , 2 ] , \"b\" : { } } } \r\n",
            r#"{"data":{"a":"\" \\ \/ \b \f \n \r \t \u00e9 \ud83d\ude00 end"}}"#,
            r#"{"data":{"a":"żółć 😀","b":"a\u0000b"}}"#,
            r#"{"data":{"a":[0,-1,1.5,-0,-0.0,1e10,1E-3,2.5e+3,0.1,123456789.123456789e-5]}}"#,
            r#"{"data":{"a":[18446744073709551615,18446744073709551616,-9223372036854775808,-9223372036854775809]}}"#,
            r#"{"data":{"a":[true,false,null]}}"#,
            // the most digits that we read in one step, and numbers that we check while we read
            r#"{"data":{"a":[9999999999999999999,-9223372036854775807,12345678901234567890,99999999999999999999]}}"#,
            // we still check the parts that we skip
            r#"{"data":{"a":1},"other":{"k":[1,{"x":"\n"}],"e":{}},"more":[]}"#,
            // not valid GraphQL responses: no data and no errors
            r#"{"data":null}"#,
            r#"{}"#,
        ];
        let shape = shape(QUERY, None);
        for body in bodies {
            assert!(
                parse_response(body.as_bytes(), &shape).is_some(),
                "the shaped parser reads {body}"
            );
            same_as_sonic(body.as_bytes(), None);
        }
    }

    /// Returns how many fields each object of `u` has space for.
    fn capacities(body: &str) -> Vec<usize> {
        same_as_sonic(body.as_bytes(), None);
        let shape = shape(QUERY, None);
        let response = parse_response(body.as_bytes(), &shape).expect("shaped");
        let Value::Object(data) = response.data else {
            panic!("data");
        };
        let Some((_, Value::Array(list))) = data.into_iter().find(|(key, _)| *key == "u") else {
            panic!("u");
        };
        list.into_iter()
            .map(|item| match item {
                Value::Object(fields) => fields.capacity(),
                _ => panic!("object"),
            })
            .collect()
    }

    #[test]
    fn reads_objects_with_the_fields_of_their_type() {
        // all the fields: __typename a1 n b1 f i
        let all = 6;
        assert_eq!(
            capacities(
                r#"{"data":{"u":[{"__typename":"A","a1":1,"n":{"x":1},"f":2,"i":3},{"__typename":"B","b1":1,"n":{"y":2}},{"__typename":"C","i":1}]}}"#
            ),
            // A also gets the fields of I. C gets only the fields of I.
            [5, 3, 2]
        );
        // a field of a type has the fields that this type selected under it
        let body = r#"{"data":{"u":[{"__typename":"A","n":{"x":1}}]}}"#;
        let shape = shape(QUERY, None);
        let data = parse_response(body.as_bytes(), &shape)
            .expect("shaped")
            .data;
        let Value::Object(data) = data else { panic!() };
        let Value::Array(u) = &data[0].1 else {
            panic!()
        };
        let Value::Object(a) = &u[0] else { panic!() };
        let Value::Object(n) = &a[1].1 else { panic!() };
        assert_eq!(n.capacity(), 1);

        // __typename is not first, a type that the shape does not know, no __typename, not a string
        assert_eq!(
            capacities(
                r#"{"data":{"u":[{"a1":1,"__typename":"A"},{"__typename":"Z","z":1},{"b1":2},{"__typename":null,"i":1}]}}"#
            ),
            [all, all, all, all]
        );
        // a type that gets other fields than it asked for still reads them
        capacities(r#"{"data":{"u":[{"__typename":"B","a1":1,"z":{"k":[1]},"b1":2}]}}"#);
    }

    #[test]
    fn leaves_invalid_and_odd_bodies_to_sonic() {
        let bodies: Vec<Vec<u8>> = [
            r#"{"data":{"a":01}}"#,
            r#"{"data":{"a":-01}}"#,
            r#"{"data":{"a":1.}}"#,
            r#"{"data":{"a":.5}}"#,
            r#"{"data":{"a":-}}"#,
            r#"{"data":{"a":1e}}"#,
            r#"{"data":{"a":1e+}}"#,
            r#"{"data":{"a":+1}}"#,
            r#"{"data":{"a":[1,]}}"#,
            r#"{"data":{"a":1,}}"#,
            r#"{"data":{"a":tru}}"#,
            r#"{"data":{"a":nul}}"#,
            r#"{"data":{"a":"\x"}}"#,
            r#"{"data":{"a":"\ud800"}}"#,
            r#"{"data":{"a\n":1}}"#,
            r#"{"data":{"a":1}} x"#,
            r#"{"data":{"a":1}"#,
            r#"{"data":{"a" 1}}"#,
            r#"{"data":{a:1}}"#,
            r#"{"data":{"a":"unterminated}}"#,
            r#"{"data":{"a":"\"#,
            r#"{"data":{"a":1},"data":{}}"#,
            r#"{"errors":null}"#,
            r#"{"errors":[{"nope":1}]}"#,
            r#"{"errors":[{"message":"\x"}]}"#,
            r#"{"data":{},"other":[01]}"#,
            r#"{"data":{},"other":{"k\n":1}}"#,
            r#"{"data":{},"other":{"k":1,}}"#,
            r#"[1]"#,
            r#""#,
        ]
        .iter()
        .map(|body| body.as_bytes().to_vec())
        .chain([
            b"{\"data\":{\"a\":\"raw \x01 control\"}}".to_vec(),
            b"{\"data\":{\"a\":\"bad \xff utf8\"}}".to_vec(),
        ])
        .collect();
        let shape = shape(QUERY, None);
        for body in &bodies {
            assert!(
                parse_response(body, &shape).is_none(),
                "{} goes to sonic",
                String::from_utf8_lossy(body)
            );
            same_as_sonic(body, None);
        }
    }

    /// `o` returns the object type O and `i` the interface I. `p` returns O on A and P on B,
    /// so without the object's `__typename` we can't tell which.
    #[test]
    fn fills_the_typenames_left_out_of_the_query() {
        let document = document(
            "{ __typename o { __typename id o { __typename id } } i { __typename id }
               u { ... on A { p { __typename id } } ... on B { p { __typename id } } } }",
        );
        let set = |types: &[&str]| HashSet::from_iter(types.iter().map(|t| t.to_string()));
        let (o, p, i) = (set(&["O"]), set(&["P"]), set(&["A", "C"]));
        let (shape, static_typenames) = ResponseShape::with_static_typenames(
            &document,
            Some("Query"),
            |name| match name {
                "O" => Some(&o),
                "P" => Some(&p),
                "I" => Some(&i),
                _ => None,
            },
            |parent, field| match (parent, field) {
                ("Query", "o") | ("O", "o") | ("A", "p") => Some("O"),
                ("B", "p") => Some("P"),
                ("Query", "i") => Some("I"),
                _ => None,
            },
            None,
        )
        .expect("a shape");

        let root = &document.operation.selection_set;
        fn selections(set: &SelectionSet, index: usize) -> &SelectionSet {
            match &set.items[index] {
                SelectionItem::Field(field) => &field.selections,
                SelectionItem::InlineFragment(fragment) => &fragment.selections,
                SelectionItem::FragmentSpread(_) => unreachable!(),
            }
        }
        let o_set = selections(root, 1);
        let u_set = selections(root, 3);
        assert!(!static_typenames.contains(root));
        assert!(static_typenames.contains(o_set));
        assert!(static_typenames.contains(selections(o_set, 2)));
        assert!(!static_typenames.contains(selections(root, 2)));
        assert!(!static_typenames.contains(selections(selections(u_set, 0), 0)));
        assert!(!static_typenames.contains(selections(selections(u_set, 1), 0)));

        let body = r#"{"data":{"__typename":"Query","o":[{"id":1,"o":{"id":2}},null],
            "i":{"__typename":"A","id":3},"u":{"p":{"__typename":"P","id":4}}}}"#;
        let mut data = parse_response(body.as_bytes(), &shape).unwrap().data;
        shape.fill_typenames(&mut data);
        assert_eq!(
            sonic_rs::to_string(&data).unwrap(),
            r#"{"__typename":"Query","o":[{"__typename":"O","id":1,"o":{"__typename":"O","id":2}},null],"i":{"__typename":"A","id":3},"u":{"p":{"__typename":"P","id":4}}}"#
        );
    }

    #[test]
    fn leaves_deep_nesting_to_sonic() {
        let shape = shape(QUERY, None);
        let deep = |depth| {
            format!(
                "{{\"data\":{{\"a\":{}{}}}}}",
                "[".repeat(depth),
                "]".repeat(depth)
            )
        };
        assert!(parse_response(deep(MAX_DEPTH as usize).as_bytes(), &shape).is_some());
        assert!(parse_response(deep(200).as_bytes(), &shape).is_none());
    }

    #[test]
    fn keeps_custom_scalars_as_raw_json_like_sonic() {
        let query = "{ a b { c d } list { x y } meta }";
        let mut paths = CustomScalarPaths::default();
        // one in an object, one inside list items, a whole list, and a leaf
        paths.insert_path(["b", "c"]);
        paths.insert_path(["list", "y"]);
        paths.insert_path(["a"]);
        paths.insert_path(["meta"]);
        let shape = shape(query, Some(&paths));

        let read: &[&str] = &[
            r#"{"data":{"meta":{ "k" : [1, "x\n", null] },"b":{"c":"2024-01-01T00:00:00Z","d":1}}}"#,
            r#"{"data":{"list":[{"x":1,"y":{"any":true}},{"y":[1,2]},{"y":null},{"y":"s\"q"},null]}}"#,
            r#"{"data":{"a":[ 1 , {"z":2} ],"meta" :   -0.5e3  ,"b":{"c":true}}}"#,
            r#"{"data":{"a":null,"meta":"","b":null,"list":[]}}"#,
            r#"{"data":{"a":"żółć 😀","meta":{}}}"#,
            // a different order, found with a search
            r#"{"data":{"list":[{"y":{"k":1},"x":2}],"meta":[1],"a":{"b":2}}}"#,
        ];
        for body in read {
            assert!(
                parse_response(body.as_bytes(), &shape).is_some(),
                "the shaped parser reads {body}"
            );
            same_as_sonic_for(query, body.as_bytes(), Some(&paths));
        }

        let left_to_sonic: &[&str] = &[
            r#"{"data":{"meta":{"k":01}}}"#,
            r#"{"data":{"meta":[1,]}}"#,
            r#"{"data":{"meta":{"k\n":1}}}"#,
        ];
        for body in left_to_sonic {
            assert!(
                parse_response(body.as_bytes(), &shape).is_none(),
                "{body} goes to sonic"
            );
            same_as_sonic_for(query, body.as_bytes(), Some(&paths));
        }
    }

    #[test]
    fn reads_custom_scalars_that_no_field_selects() {
        let mut paths = CustomScalarPaths::default();
        paths.insert_path(["unknown"]);
        paths.insert_path(["b", "unknown"]);
        paths.insert_path(["deep", "x"]);
        let shape = shape(QUERY, Some(&paths));
        for body in [
            r#"{"data":{"unknown":{"k":1}}}"#,
            r#"{"data":{"b":{"c":1,"unknown":[1]}}}"#,
            // a path that goes deeper than the key
            r#"{"data":{"deep":{"y":{"k":1},"x":{"k":[1]}}}}"#,
            r#"{"data":{"deep":[{"x":"s"},null,{"x":{}}],"unknown":null}}"#,
            // other keys that the shape does not know, also next to them
            r#"{"data":{"a":1,"other":2,"b":{"c":1,"d":2,"z":3},"list":[{"x":1,"y":2}]}}"#,
        ] {
            assert!(parse_response(body.as_bytes(), &shape).is_some(), "{body}");
            same_as_sonic(body.as_bytes(), Some(&paths));
        }
    }

    #[test]
    fn reads_a_field_that_is_an_object_for_one_type_and_not_for_another() {
        // On A, the value of `f` is an object. On B, it is a scalar. The custom scalar is under the
        // `f` of A.
        let query = "{ u { __typename ... on A { f { a } } ... on B { f } } }";
        let mut paths = CustomScalarPaths::default();
        paths.insert_path(["u", "f", "a"]);
        let shape = shape(query, Some(&paths));
        for body in [
            r#"{"data":{"u":[{"__typename":"A","f":{"a":{"raw":1}}}]}}"#,
            r#"{"data":{"u":[{"__typename":"B","f":"s"}]}}"#,
            r#"{"data":{"u":[{"__typename":"B","f":{"k":{"x":1}}}]}}"#,
            // the custom scalar under B, where the fields of B do not have it
            r#"{"data":{"u":[{"__typename":"B","f":{"a":{"raw":1}}}]}}"#,
        ] {
            assert!(parse_response(body.as_bytes(), &shape).is_some(), "{body}");
            same_as_sonic_for(query, body.as_bytes(), Some(&paths));
        }
    }

    #[test]
    fn finds_fields_of_wide_objects_in_any_order() {
        // short keys, and long keys with the same first 8 bytes
        for name in ["field", "same_first_bytes_"] {
            finds_fields_in_any_order(name);
        }
    }

    fn finds_fields_in_any_order(name: &str) {
        let fields: Vec<String> = (0..40).map(|i| format!("{name}{i}")).collect();
        let query = format!("{{ wide {{ {} }} }}", fields.join(" "));
        let object = |order: &mut dyn Iterator<Item = &String>| {
            let entries: Vec<String> = order.map(|key| format!(r#""{key}":1"#)).collect();
            format!("{{{}}}", entries.join(","))
        };
        let body = format!(
            r#"{{"data":{{"wide":[{},{}]}}}}"#,
            object(&mut fields.iter()),
            object(&mut fields.iter().rev())
        );
        let shape = shape(&query, None);
        assert!(parse_response(body.as_bytes(), &shape).is_some());
        same_as_sonic_for(&query, body.as_bytes(), None);
    }

    #[test]
    fn has_no_shape_for_keys_too_long_to_index() {
        let query = format!("{{ {} }}", "k".repeat(300));
        let document = parse_operation(&query);
        let Some(query::Definition::Operation(query::OperationDefinition::SelectionSet(set))) =
            document.definitions.into_iter().next()
        else {
            panic!("a selection set")
        };
        let document = Document {
            operation: OperationDefinition {
                name: None,
                operation_kind: None,
                selection_set: set.into(),
                variable_definitions: None,
            },
            fragments: Vec::new(),
        };
        assert!(ResponseShape::new(&document, |_| None, None).is_none());
    }
}
