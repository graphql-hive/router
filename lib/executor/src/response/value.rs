use std::{
    fmt::Display,
    hash::{Hash, Hasher},
};

/// A value in the response tree.
///
/// `Object` is **dense and slot-addressed**: the values sit at the index their response key
/// occupies in the `ResponseShape` for that position, and the keys themselves are not stored
/// at all. Every consumer — projection, merge, traversal, `requires` — already knows which
/// position it is looking at, so the key set is implied and never has to be carried, sorted,
/// or searched.
///
/// A slot the subgraph did not answer holds `Value::Absent`, which is distinct from a `null`
/// it actually answered. Almost everything treats the two alike — projection emits `null` and
/// propagates for both, neither overwrites anything in a merge, demand control charges the
/// base cost for both — but `requires` does not: an absent field is left out of the
/// representation, while an explicit `null` is sent as `null`.
#[derive(Debug, Clone, Default)]
pub enum Value<'a> {
    /// A slot no response has filled in. `Value::default()`, so taking a slot empties it.
    #[default]
    Absent,
    Null,
    F64(f64),
    I64(i64),
    U64(u64),
    Bool(bool),
    /// Borrowed straight out of the response buffer — the overwhelming majority of strings,
    /// since a JSON string only needs rewriting if it contains escapes.
    String(&'a str),
    /// The rare string that had to be unescaped, or one the router synthesized. A separate
    /// variant rather than a `Cow` because `Cow<str>` is 24 bytes and would set the size of
    /// the whole enum, while `Box<str>` is 16 like everything else here.
    OwnedString(Box<str>),
    RawJson(&'a str),
    /// Boxed slices, not `Vec`s: a `Vec` carries a capacity field it never needs here — both
    /// are built once and then only mutated in place — and dropping that word is what brings
    /// `Value` from 32 bytes to 24. Every copy, drop and `memmove` of a response tree shrinks
    /// by a quarter with it, and those were 15% of on-CPU time in a load profile.
    Array(Box<[Value<'a>]>),
    Object(Box<[Value<'a>]>),
}

impl<'a> AsRef<Value<'a>> for Value<'a> {
    fn as_ref(&self) -> &Value<'a> {
        self
    }
}

impl Hash for Value<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Value::Absent => (-1i8).hash(state),
            Value::Null => 0.hash(state),
            Value::F64(f) => f.to_bits().hash(state),
            Value::I64(i) => i.hash(state),
            Value::U64(u) => u.hash(state),
            Value::Bool(b) => b.hash(state),
            Value::String(s) => s.hash(state),
            Value::OwnedString(s) => s.hash(state),
            Value::RawJson(raw) => raw.hash(state),
            Value::Array(arr) => arr.hash(state),
            Value::Object(slots) => slots.hash(state),
        }
    }
}

impl<'a> Value<'a> {
    /// Takes the entity list out of a subgraph response. `slot` is where `_entities` (or a
    /// batch alias) sits in the fetch's response shape.
    pub fn take_entities_at(&mut self, slot: usize) -> Option<Box<[Value<'a>]>> {
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
        static ABSENT: Value<'static> = Value::Absent;
        self.slot(slot).unwrap_or(&ABSENT)
    }

    /// An object with every slot empty, ready for a shape of `len` fields.
    pub fn empty_object(len: usize) -> Value<'a> {
        Value::Object(vec![Value::Absent; len].into_boxed_slice())
    }

    pub fn as_object(&self) -> Option<&[Value<'a>]> {
        match self {
            Value::Object(slots) => Some(slots),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            Value::OwnedString(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_raw_json(&self) -> Option<&str> {
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
            Value::OwnedString(s) => write!(f, "{:?}", s),
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

    #[test]
    fn an_unfilled_slot_is_absent_but_still_projects_as_null() {
        let object = Value::empty_object(3);
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
        let object = Value::Object(vec![Value::Null, Value::Absent].into_boxed_slice());
        assert!(!object.slot(0).unwrap().is_absent());
        assert!(object.slot(0).unwrap().is_null());
        assert!(object.slot(1).unwrap().is_absent());
    }

    #[test]
    fn take_entities_at_empties_the_slot() {
        let mut object = Value::Object(
            vec![
                Value::Null,
                Value::Array(vec![Value::I64(1), Value::I64(2)].into_boxed_slice()),
            ]
            .into_boxed_slice(),
        );
        let entities = object.take_entities_at(1).expect("entities");
        assert_eq!(entities.len(), 2);
        assert!(object.slot(1).unwrap().is_absent());
        assert!(object.take_entities_at(1).is_none());
        assert!(object.take_entities_at(0).is_none());
    }
}
