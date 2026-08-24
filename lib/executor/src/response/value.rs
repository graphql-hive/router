use std::fmt::Display;

use bumpalo::Bump;

/// A value in the response tree.
///
/// `Object` is **dense and slot-addressed**: the values sit at the index their response key
/// occupies in the `ResponseShape` for that position, and the keys themselves are not stored
/// at all. Every consumer — projection, merge, traversal, `requires` — already knows which
/// position it is looking at, so the key set is implied and never has to be carried, sorted,
/// or searched.
///
/// Nothing here owns an allocation. Every string borrows the response bytes or an arena, and
/// both slice variants borrow an arena, so a `Value` tree has **no `Drop` glue at all**:
/// tearing down a response is dropping the arena, not walking the tree. Freeing response
/// trees node by node was 3.3% of on-CPU time in a load profile, and the allocator another
/// 2.5% on top.
///
/// A slot the subgraph did not answer holds `Value::Absent`, which is distinct from a `null`
/// it actually answered. Almost everything treats the two alike — projection emits `null` and
/// propagates for both, neither overwrites anything in a merge, demand control charges the
/// base cost for both — but `requires` does not: an absent field is left out of the
/// representation, while an explicit `null` is sent as `null`.
#[derive(Debug, Default)]
pub enum Value<'a> {
    /// A slot no response has filled in. `Value::default()`, so taking a slot empties it.
    #[default]
    Absent,
    Null,
    F64(f64),
    I64(i64),
    U64(u64),
    Bool(bool),
    /// Borrowed from the response buffer — the overwhelming majority of strings, since a JSON
    /// string only needs rewriting if it contains escapes — or from the arena, for the rare
    /// one that had to be unescaped and for anything the router synthesized. The two used to
    /// be separate variants because one of them owned a `Box<str>`; with the arena they are
    /// the same borrowed `&str`, and every match on a string is one arm shorter.
    String(&'a str),
    RawJson(&'a str),
    /// Arena slices, exclusive so a merge can write into them in place. Both are sized once
    /// and never grow, so there is no capacity word to carry: 16 bytes, which keeps `Value`
    /// at 24.
    Array(&'a mut [Value<'a>]),
    Object(&'a mut [Value<'a>]),
}

/// The response tree the executor builds, all the way from a subgraph's bytes to projection.
///
/// The lifetime is not a borrow of anything the compiler can see: the response buffers and
/// arenas these values point into are owned by the execution context, which outlives them, and
/// the borrow is laundered where it is created (see `ResponseArena`). While `Value` was
/// covariant this could be spelled with the executor's own `'exec` and narrowed on the way in;
/// `&'a mut [Value<'a>]` is invariant, so one concrete lifetime has to serve everywhere, and
/// `'static` is the honest one — nothing shorter is really being borrowed.
pub type ResponseTree = Value<'static>;

impl<'a> AsRef<Value<'a>> for Value<'a> {
    fn as_ref(&self) -> &Value<'a> {
        self
    }
}

impl<'a> Value<'a> {
    /// Takes the entity list out of a subgraph response. `slot` is where `_entities` (or a
    /// batch alias) sits in the fetch's response shape.
    pub fn take_entities_at(&mut self, slot: usize) -> Option<&'a mut [Value<'a>]> {
        match self.slot_mut(slot).map(std::mem::take) {
            Some(Value::Array(entities)) => Some(entities),
            _ => None,
        }
    }

    #[inline]
    pub fn slot(&self, slot: usize) -> Option<&Value<'a>> {
        match self {
            Value::Object(slots) => slots.get(slot),
            _ => None,
        }
    }

    #[inline]
    pub fn slot_mut(&mut self, slot: usize) -> Option<&mut Value<'a>> {
        match self {
            Value::Object(slots) => slots.get_mut(slot),
            _ => None,
        }
    }

    /// The slot's value, or `Absent` when this is not an object or the slot is out of range.
    #[inline]
    pub fn slot_or_absent(&self, slot: usize) -> &Value<'a> {
        // `Value` has no `Drop` and no interior mutability, so this promotes to a `'static`
        // constant rather than a temporary.
        self.slot(slot).unwrap_or(&Value::Absent)
    }

    /// An object with every slot empty, ready for a shape of `len` fields.
    pub fn empty_object(arena: &'a Bump, len: usize) -> Value<'a> {
        Value::Object(arena.alloc_slice_fill_default(len))
    }

    /// Copies this value into `arena`, deeply.
    ///
    /// Not a `Clone` impl: a `Value` holds exclusive slices, so a copy needs somewhere to put
    /// them, and having to name that place is the point. Scalars copy without allocating.
    pub fn copy_into<'b>(&self, arena: &'b Bump) -> Value<'b>
    where
        'a: 'b,
    {
        match self {
            Value::Absent => Value::Absent,
            Value::Null => Value::Null,
            Value::F64(n) => Value::F64(*n),
            Value::I64(n) => Value::I64(*n),
            Value::U64(n) => Value::U64(*n),
            Value::Bool(b) => Value::Bool(*b),
            Value::String(s) => Value::String(s),
            Value::RawJson(raw) => Value::RawJson(raw),
            Value::Array(items) => Value::Array(copy_slice_into(items, arena)),
            Value::Object(slots) => Value::Object(copy_slice_into(slots, arena)),
        }
    }

    pub fn as_object(&self) -> Option<&[Value<'a>]> {
        match self {
            Value::Object(slots) => Some(slots),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&'a str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_raw_json(&self) -> Option<&'a str> {
        match self {
            Value::RawJson(raw) => Some(raw),
            _ => None,
        }
    }

    /// True for a value that projects as `null`: one the subgraph answered as null, and one
    /// no response filled in. Use `is_absent` where the difference matters.
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null | Value::Absent)
    }

    /// True only when no response ever filled this slot.
    pub fn is_absent(&self) -> bool {
        matches!(self, Value::Absent)
    }

    pub fn is_object(&self) -> bool {
        matches!(self, Value::Object(_))
    }
}

fn copy_slice_into<'a: 'b, 'b>(values: &[Value<'a>], arena: &'b Bump) -> &'b mut [Value<'b>] {
    arena.alloc_slice_fill_iter(values.iter().map(|value| value.copy_into(arena)))
}

/// Diagnostics only. Objects have no keys of their own, so slots are printed positionally —
/// enough to read a value in a log or a panic message, not a JSON encoder. The response is
/// written by `projection`, which has the shape and therefore the names.
impl Display for Value<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Absent => write!(f, "<absent>"),
            Value::Null => write!(f, "null"),
            Value::Bool(b) => write!(f, "{}", b),
            Value::String(s) => write!(f, "{:?}", s),
            Value::RawJson(raw) => write!(f, "{}", raw),
            Value::F64(n) => write!(f, "{}", n),
            Value::U64(n) => write!(f, "{}", n),
            Value::I64(n) => write!(f, "{}", n),
            Value::Array(arr) => {
                write!(f, "[")?;
                for (i, v) in arr.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", v)?;
                }
                write!(f, "]")
            }
            Value::Object(slots) => {
                write!(f, "{{")?;
                for (slot, v) in slots.iter().enumerate() {
                    if slot > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}: {}", slot, v)?;
                }
                write!(f, "}}")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Value;
    use bumpalo::Bump;

    #[test]
    fn a_value_tree_has_no_drop_glue() {
        // The whole point of the arena: tearing down a response is dropping the arena, not
        // walking the tree.
        assert!(!std::mem::needs_drop::<Value<'_>>());
        assert_eq!(std::mem::size_of::<Value<'_>>(), 24);
    }

    #[test]
    fn an_unfilled_slot_is_absent_but_still_projects_as_null() {
        let arena = Bump::new();
        let object = Value::empty_object(&arena, 3);
        assert!(object.slot(0).unwrap().is_absent());
        assert!(object.slot(0).unwrap().is_null());
        // Past the end reads as absent too.
        assert!(object.slot(3).is_none());
        assert!(object.slot_or_absent(3).is_absent());
    }

    #[test]
    fn an_answered_null_is_not_absent() {
        // `requires` sends an explicit null but leaves an absent field out, so the two must
        // stay distinguishable.
        let arena = Bump::new();
        let object = Value::Object(arena.alloc_slice_fill_iter([Value::Null, Value::Absent]));
        assert!(!object.slot(0).unwrap().is_absent());
        assert!(object.slot(0).unwrap().is_null());
        assert!(object.slot(1).unwrap().is_absent());
    }

    #[test]
    fn take_entities_at_empties_the_slot() {
        let arena = Bump::new();
        let mut object = Value::Object(arena.alloc_slice_fill_iter([
            Value::Null,
            Value::Array(arena.alloc_slice_fill_iter([Value::I64(1), Value::I64(2)])),
        ]));
        let entities = object.take_entities_at(1).expect("entities");
        assert_eq!(entities.len(), 2);
        assert!(object.slot(1).unwrap().is_absent());
        assert!(object.take_entities_at(1).is_none());
        assert!(object.take_entities_at(0).is_none());
    }

    #[test]
    fn copy_into_is_deep() {
        let arena = Bump::new();
        let source = Value::Object(arena.alloc_slice_fill_iter([
            Value::String("a"),
            Value::Array(arena.alloc_slice_fill_iter([Value::I64(1)])),
        ]));
        let mut copy = source.copy_into(&arena);
        // Writing through the copy must not reach the original.
        if let Some(Value::Array(items)) = copy.slot_mut(1) {
            items[0] = Value::I64(9);
        }
        assert_eq!(source.to_string(), r#"{0: "a", 1: [1]}"#);
        assert_eq!(copy.to_string(), r#"{0: "a", 1: [9]}"#);
    }
}
