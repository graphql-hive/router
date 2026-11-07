//! Where a request's response data lives: plain `Copy` slots in a few `Vec`s, pointing at the
//! subgraph bodies by index. Nothing to drop, so clearing it is O(1) and keeps the capacity for
//! the next request.

use std::cell::RefCell;

use ahash::HashMap as AHashMap;
use bytes::Bytes;

use super::compile::Layout;
use crate::executor::json_writer::write_and_escape_string;

/// One JSON value. Scalars are byte ranges of a subgraph body, as the subgraph wrote them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    /// Not in the response
    Missing,
    Null,
    True,
    False,
    /// A number, a string with its quotes, or any JSON kept as it is (a custom scalar)
    Raw(Raw),
    Object {
        layout: u32,
        first: u32,
    },
    List {
        start: u32,
        len: u32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Raw {
    pub body: u32,
    pub start: u32,
    pub len: u32,
}

/// The storage of one request. Pooled per thread, see [`with_arena`].
#[derive(Default)]
pub struct Arena {
    /// The subgraph bodies, alive for the request
    pub bodies: Vec<Bytes>,
    /// Object blocks, `width` slots each
    pub slots: Vec<Slot>,
    /// Finished list items, contiguous per list
    pub items: Vec<Slot>,
    /// List items being parsed, one run per open list
    pub scratch: Vec<Slot>,
    /// Representation bytes of the fetch being prepared
    pub reps: Vec<u8>,
    /// Dedupe: hash of a written representation, its entity index
    pub seen: AHashMap<u64, u32>,
    /// Spots found by a path walk
    pub spots: Vec<Slot>,
    /// Scratch space of a path walk
    pub frontier: Vec<Slot>,
    /// Dedupe: where each representation is in `reps`, to check a hash match byte for byte
    pub ranges: Vec<(u32, u32)>,
    /// Dedupe: fingerprint of what a spot's representation reads, its entity index
    pub prints: AHashMap<u64, u32>,
    /// Spare lists for the spots of a fetch's aliases, kept across requests
    pub targets: Vec<Vec<(Slot, u32)>>,
    /// Entity reuse: fingerprint of an entity already in the response, its object and source
    pub known: AHashMap<u64, (u32, u32)>,
    /// Passthrough checks that failed in the body being parsed, by `Shape::verifies` index
    pub failed: Vec<bool>,
}

/// Arenas kept for the next request on this thread. A request takes one and gives it back
/// when it ends, wherever it ends.
const POOL_SIZE: usize = 4;
/// An arena that grew past this many slots isn't kept, so one huge response doesn't pin its
/// memory for the life of the worker.
const MAX_RETAINED_SLOTS: usize = 1 << 20;

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
        if self.slots.capacity() + self.items.capacity() > MAX_RETAINED_SLOTS
            || self.reps.capacity() > MAX_RETAINED_SLOTS * 16
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

    pub fn give_back_targets(&mut self, mut targets: Vec<(Slot, u32)>) {
        if self.targets.len() < 16 {
            targets.clear();
            self.targets.push(targets);
        }
    }

    pub fn clear(&mut self) {
        self.bodies.clear();
        self.slots.clear();
        self.items.clear();
        self.scratch.clear();
        self.reps.clear();
        self.seen.clear();
        self.spots.clear();
        self.frontier.clear();
        self.ranges.clear();
        self.prints.clear();
        self.known.clear();
        self.failed.clear();
    }

    #[inline]
    pub fn raw(&self, raw: Raw) -> &[u8] {
        &self.bodies[raw.body as usize][raw.start as usize..(raw.start + raw.len) as usize]
    }

    /// A new block of `width` missing slots
    #[inline]
    pub fn object(&mut self, width: usize) -> u32 {
        let first = self.slots.len();
        self.slots.resize(first + width, Slot::Missing);
        first as u32
    }

    #[inline]
    pub fn list_items(&self, start: u32, len: u32) -> &[Slot] {
        &self.items[start as usize..(start + len) as usize]
    }

    /// The string in a slot, without its quotes, when it has no escapes. Type names and enum
    /// values never have any.
    #[inline]
    pub fn plain_str(&self, slot: Slot) -> Option<&str> {
        let Slot::Raw(raw) = slot else {
            return None;
        };
        let bytes = self.raw(raw);
        let inner = bytes.strip_prefix(b"\"")?.strip_suffix(b"\"")?;
        if inner.contains(&b'\\') {
            return None;
        }
        // SAFETY: the parser checked every string is UTF-8
        Some(unsafe { std::str::from_utf8_unchecked(inner) })
    }

    /// Like [`Self::plain_str`], but decodes escapes when there are any.
    pub fn string(&self, slot: Slot) -> Option<std::borrow::Cow<'_, str>> {
        if let Some(plain) = self.plain_str(slot) {
            return Some(plain.into());
        }
        let Slot::Raw(raw) = slot else {
            return None;
        };
        let bytes = self.raw(raw);
        if bytes.first() != Some(&b'"') {
            return None;
        }
        sonic_rs::from_slice::<String>(bytes).ok().map(Into::into)
    }

    /// Merges `source` into the value `target` holds, with `deep_merge`'s rules: a null or
    /// missing source changes nothing, objects merge slot by slot, lists zip by index, anything
    /// else replaces the target. Returns the new value of the target when it changes.
    ///
    /// The source's objects are shared, not copied: an entity's nested objects end up in every
    /// spot that got the entity.
    pub fn merge(&mut self, layouts: &[Layout], target: Slot, source: Slot) -> Option<Slot> {
        match (target, source) {
            (_, Slot::Missing | Slot::Null) => None,
            (
                Slot::Object {
                    layout: target_layout,
                    first: target_first,
                },
                Slot::Object { layout, first },
            ) if target_layout == layout => {
                if target_first != first {
                    self.merge_object(layouts, layout, target_first, first);
                }
                None
            }
            (Slot::List { start: t, len: tl }, Slot::List { start: s, len: sl }) => {
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
    pub fn merge_object(&mut self, layouts: &[Layout], layout: u32, target: u32, source: u32) {
        let width = layouts[layout as usize].width() as u32;
        for i in 0..width {
            let source_slot = self.slots[(source + i) as usize];
            if source_slot == Slot::Missing {
                continue;
            }
            let target_slot = self.slots[(target + i) as usize];
            // a key the target doesn't have is added, even as null: `deep_merge` does, and
            // `project_requires` tells a null field from a missing one
            if target_slot == Slot::Missing {
                self.slots[(target + i) as usize] = source_slot;
            } else if let Some(value) = self.merge(layouts, target_slot, source_slot) {
                self.slots[(target + i) as usize] = value;
            }
        }
    }

    /// Writes a value as it is, objects with every key they have, in their layout's order.
    pub fn write_whole(&self, layouts: &[Layout], slot: Slot, buffer: &mut Vec<u8>) {
        match slot {
            Slot::Missing | Slot::Null => buffer.extend_from_slice(b"null"),
            Slot::True => buffer.extend_from_slice(b"true"),
            Slot::False => buffer.extend_from_slice(b"false"),
            Slot::Raw(raw) => buffer.extend_from_slice(self.raw(raw)),
            Slot::Object { layout, first } => {
                buffer.push(b'{');
                let mut comma = false;
                for (i, key) in layouts[layout as usize].keys.iter().enumerate() {
                    let slot = self.slots[first as usize + i];
                    if slot == Slot::Missing {
                        continue;
                    }
                    if comma {
                        buffer.push(b',');
                    }
                    comma = true;
                    write_and_escape_string(buffer, key);
                    buffer.push(b':');
                    self.write_whole(layouts, slot, buffer);
                }
                buffer.push(b'}');
            }
            Slot::List { start, len } => {
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
