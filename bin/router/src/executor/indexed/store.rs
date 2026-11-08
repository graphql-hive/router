//! Where a request's response data lives: plain `Copy` cells in a few `Vec`s, pointing at the
//! subgraph bodies by index. Nothing to drop, so clearing it is O(1) and keeps the capacity for
//! the next request.

use std::cell::RefCell;

use ahash::HashMap as AHashMap;
use bytes::Bytes;

use super::compile::ObjectLayout;
use crate::executor::json_writer::write_and_escape_string;

/// One JSON value. Scalars are byte ranges of a subgraph body, as the subgraph wrote them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueCell {
    /// Not in the response
    Missing,
    Null,
    True,
    False,
    /// A number, a string with its quotes, or any JSON kept as it is (a custom scalar)
    RawJson(JsonSpan),
    Object {
        layout: u32,
        first: u32,
    },
    List {
        start: u32,
        len: u32,
    },
}

// A cell is copied everywhere and stored by the million: keep it at 16 bytes
const _: () = assert!(std::mem::size_of::<ValueCell>() == 16);

/// [`JsonSpan::body`] of a value a plugin set: its bytes are in [`Arena::edits`]
pub const EDITS_BODY: u32 = u32::MAX;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JsonSpan {
    pub body: u32,
    pub start: u32,
    pub len: u32,
}

/// A JSON string without its quotes, when it has no escapes.
///
/// # Safety
///
/// `json` must be bytes the parser read: it checked every string in them is UTF-8.
#[inline]
pub unsafe fn plain_str_of(json: &[u8]) -> Option<&str> {
    let inner = json.strip_prefix(b"\"")?.strip_suffix(b"\"")?;
    if inner.contains(&b'\\') {
        return None;
    }
    // SAFETY: see above
    Some(unsafe { std::str::from_utf8_unchecked(inner) })
}

/// The storage of one request. Pooled per thread, see [`with_arena`].
#[derive(Default)]
pub struct Arena {
    /// The subgraph bodies, alive for the request
    pub bodies: Vec<Bytes>,
    /// Object blocks, `width` cells each
    pub cells: Vec<ValueCell>,
    /// Finished list items, contiguous per list
    pub items: Vec<ValueCell>,
    /// List items being parsed, one run per open list
    pub scratch: Vec<ValueCell>,
    /// Representation bytes of the fetch being prepared
    pub reps: Vec<u8>,
    /// Dedupe: hash of a written representation, its entity index
    pub seen: AHashMap<u64, u32>,
    /// Values found by a path walk
    pub found: Vec<ValueCell>,
    /// Scratch space of a path walk
    pub frontier: Vec<ValueCell>,
    /// Dedupe: where each representation is in `reps`, to check a hash match byte for byte
    pub ranges: Vec<(u32, u32)>,
    /// Dedupe: fingerprint of what an entity target's representation reads, its entity index
    pub prints: AHashMap<u64, u32>,
    /// Spare entry lists for the entity target maps of a fetch's aliases, kept across requests
    pub targets: Vec<Vec<(ValueCell, u32)>>,
    /// Entity reuse: fingerprint of an entity already in the response, its object and source
    pub known: AHashMap<u64, (u32, u32)>,
    /// Passthrough checks that failed in the body being parsed, by `FetchShape::verifies` index
    pub failed: Vec<bool>,
    /// Values plugins set, as JSON, see [`EDITS_BODY`]. Kept across requests like the rest.
    pub edits: Vec<u8>,
}

/// Arenas kept for the next request on this thread. A request takes one and gives it back
/// when it ends, wherever it ends.
const POOL_SIZE: usize = 4;
/// An arena that grew past this many cells isn't kept, so one huge response doesn't pin its
/// memory for the life of the worker.
const MAX_RETAINED_CELLS: usize = 1 << 20;

thread_local! {
    static POOL: RefCell<Vec<Arena>> = const { RefCell::new(Vec::new()) };
}

impl Arena {
    pub fn take() -> Arena {
        POOL.with(|pool| pool.borrow_mut().pop())
            .unwrap_or_default()
    }

    pub fn give_back(mut self) {
        self.clear();
        if self.cells.capacity() + self.items.capacity() > MAX_RETAINED_CELLS
            || self.reps.capacity() > MAX_RETAINED_CELLS * 16
            || self.edits.capacity() > MAX_RETAINED_CELLS * 16
        {
            return;
        }
        POOL.with(|pool| {
            let mut pool = pool.borrow_mut();
            if pool.len() < POOL_SIZE {
                pool.push(self);
            }
        });
    }

    pub fn give_back_targets(&mut self, mut targets: Vec<(ValueCell, u32)>) {
        if self.targets.len() < 16 {
            targets.clear();
            self.targets.push(targets);
        }
    }

    pub fn clear(&mut self) {
        self.bodies.clear();
        self.cells.clear();
        self.items.clear();
        self.scratch.clear();
        self.reps.clear();
        self.seen.clear();
        self.found.clear();
        self.frontier.clear();
        self.ranges.clear();
        self.prints.clear();
        self.known.clear();
        self.failed.clear();
        self.edits.clear();
    }

    /// A body by its index, or the values plugins set for [`EDITS_BODY`]
    #[inline]
    pub fn body(&self, body: u32) -> &[u8] {
        if body == EDITS_BODY {
            &self.edits
        } else {
            &self.bodies[body as usize]
        }
    }

    #[inline]
    pub fn json(&self, span: JsonSpan) -> &[u8] {
        &self.body(span.body)[span.start as usize..(span.start + span.len) as usize]
    }

    /// A new block of `width` missing cells
    #[inline]
    pub fn object(&mut self, width: usize) -> u32 {
        let first = self.cells.len();
        self.cells.resize(first + width, ValueCell::Missing);
        first as u32
    }

    #[inline]
    pub fn list_items(&self, start: u32, len: u32) -> &[ValueCell] {
        &self.items[start as usize..(start + len) as usize]
    }

    /// The string in a cell, without its quotes, when it has no escapes. Type names and enum
    /// values never have any.
    #[inline]
    pub fn plain_str(&self, cell: ValueCell) -> Option<&str> {
        let ValueCell::RawJson(span) = cell else {
            return None;
        };
        // SAFETY: the bytes of a cell are what the parser read
        unsafe { plain_str_of(self.json(span)) }
    }

    /// Like [`Self::plain_str`], but decodes escapes when there are any.
    pub fn string(&self, cell: ValueCell) -> Option<std::borrow::Cow<'_, str>> {
        if let Some(plain) = self.plain_str(cell) {
            return Some(plain.into());
        }
        let ValueCell::RawJson(span) = cell else {
            return None;
        };
        let bytes = self.json(span);
        if bytes.first() != Some(&b'"') {
            return None;
        }
        sonic_rs::from_slice::<String>(bytes).ok().map(Into::into)
    }

    /// Merges `source` into the value `target` holds, with `deep_merge`'s rules: a null or
    /// missing source changes nothing, objects merge cell by cell, lists zip by index, anything
    /// else replaces the target. Returns the new value of the target when it changes.
    ///
    /// The source's objects are shared, not copied: an entity's nested objects end up in every
    /// entity target that got the entity.
    pub fn merge(
        &mut self,
        layouts: &[ObjectLayout],
        target: ValueCell,
        source: ValueCell,
    ) -> Option<ValueCell> {
        match (target, source) {
            (_, ValueCell::Missing | ValueCell::Null) => None,
            (
                ValueCell::Object {
                    layout: target_layout,
                    first: target_first,
                },
                ValueCell::Object { layout, first },
            ) if target_layout == layout => {
                if target_first != first {
                    self.merge_object(layouts, layout, target_first, first);
                }
                None
            }
            (ValueCell::List { start: t, len: tl }, ValueCell::List { start: s, len: sl }) => {
                for i in 0..tl.min(sl) {
                    let (target, source) =
                        (self.items[(t + i) as usize], self.items[(s + i) as usize]);
                    if let Some(value) = self.merge(layouts, target, source) {
                        self.items[(t + i) as usize] = value;
                    }
                }
                None
            }
            _ => Some(source),
        }
    }

    /// Merges the block at `source` into the block at `target`, both of `layout`.
    pub fn merge_object(
        &mut self,
        layouts: &[ObjectLayout],
        layout: u32,
        target: u32,
        source: u32,
    ) {
        let width = layouts[layout as usize].width() as u32;
        for i in 0..width {
            let source_cell = self.cells[(source + i) as usize];
            if source_cell == ValueCell::Missing {
                continue;
            }
            let target_cell = self.cells[(target + i) as usize];
            // a key the target doesn't have is added, even as null: `deep_merge` does, and
            // `project_requires` tells a null field from a missing one
            if target_cell == ValueCell::Missing {
                self.cells[(target + i) as usize] = source_cell;
            } else if let Some(value) = self.merge(layouts, target_cell, source_cell) {
                self.cells[(target + i) as usize] = value;
            }
        }
    }

    /// Writes a value as it is, objects with every key they have, in their layout's order.
    pub fn write_whole(&self, layouts: &[ObjectLayout], cell: ValueCell, buffer: &mut Vec<u8>) {
        match cell {
            ValueCell::Missing | ValueCell::Null => buffer.extend_from_slice(b"null"),
            ValueCell::True => buffer.extend_from_slice(b"true"),
            ValueCell::False => buffer.extend_from_slice(b"false"),
            ValueCell::RawJson(span) => buffer.extend_from_slice(self.json(span)),
            ValueCell::Object { layout, first } => {
                buffer.push(b'{');
                let mut comma = false;
                for (i, key) in layouts[layout as usize].keys.iter().enumerate() {
                    let cell = self.cells[first as usize + i];
                    if cell == ValueCell::Missing {
                        continue;
                    }
                    if comma {
                        buffer.push(b',');
                    }
                    comma = true;
                    write_and_escape_string(buffer, key);
                    buffer.push(b':');
                    self.write_whole(layouts, cell, buffer);
                }
                buffer.push(b'}');
            }
            ValueCell::List { start, len } => {
                buffer.push(b'[');
                for i in 0..len {
                    if i > 0 {
                        buffer.push(b',');
                    }
                    self.write_whole(layouts, self.items[(start + i) as usize], buffer);
                }
                buffer.push(b']');
            }
        }
    }
}

/// A value in the cells, read the way demand control counts the actual cost of a response
/// (see `CostData`). Objects find a field by its response key in their layout.
#[derive(Clone, Copy)]
pub struct CellView<'a> {
    pub arena: &'a Arena,
    pub layouts: &'a [ObjectLayout],
    pub cell: ValueCell,
}

impl<'a> crate::executor::execution::demand_control::CostData<'a> for CellView<'a> {
    fn field(self, key: &str) -> Option<Self> {
        let ValueCell::Object { layout, first } = self.cell else {
            return None;
        };
        // ponytail: a linear search of the layout's keys per field, only while demand control
        // counts the actual cost; resolve the cost plan to field indexes when that shows in a profile
        let field_index = self.layouts[layout as usize].field_index(key);
        if field_index == super::compile::NO_FIELD {
            return None;
        }
        match self.arena.cells[(first + field_index as u32) as usize] {
            ValueCell::Missing => None,
            cell => Some(Self { cell, ..self }),
        }
    }

    fn items(self) -> Option<impl Iterator<Item = Self>> {
        let ValueCell::List { start, len } = self.cell else {
            return None;
        };
        Some(
            self.arena
                .list_items(start, len)
                .iter()
                .map(move |&cell| Self { cell, ..self }),
        )
    }

    fn is_null(self) -> bool {
        self.cell == ValueCell::Null
    }

    fn typename(self) -> Option<&'a str> {
        let ValueCell::Object { layout, first } = self.cell else {
            return None;
        };
        let field_index = self.layouts[layout as usize].typename;
        if field_index == super::compile::NO_FIELD {
            return None;
        }
        self.arena
            .plain_str(self.arena.cells[(first + field_index as u32) as usize])
    }
}
