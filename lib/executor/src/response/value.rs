use core::fmt;
use hive_router_query_planner::ast::selection_item::SelectionItem;
use serde::{
    de::{self, Deserializer, MapAccess, SeqAccess, Visitor},
    ser::{SerializeMap, SerializeSeq},
};
use sonic_rs::{JsonNumberTrait, ValueRef};
use std::{
    fmt::Display,
    hash::{Hash, Hasher},
};
use xxhash_rust::xxh3::Xxh3;

use crate::{introspection::schema::PossibleTypes, utils::consts::TYPENAME_FIELD_NAME};

#[derive(Clone)]
pub enum Value<'a> {
    Null,
    F64(f64),
    I64(i64),
    U64(u64),
    Bool(bool),
    String(&'a str),
    Array(Vec<Value<'a>>),
    Object(Vec<(&'a str, Value<'a>)>),
}

impl Hash for Value<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Value::Null => 0.hash(state),
            Value::F64(f) => f.to_bits().hash(state),
            Value::I64(i) => i.hash(state),
            Value::U64(u) => u.hash(state),
            Value::Bool(b) => b.hash(state),
            Value::String(s) => s.hash(state),
            Value::Array(arr) => arr.hash(state),
            Value::Object(obj) => obj.hash(state),
        }
    }
}

impl<'a> Value<'a> {
    pub fn take_entities<'b: 'a>(&'a mut self) -> Option<Vec<Value<'a>>> {
        match self {
            Value::Object(data) => {
                if let Ok(entities_idx) = data.binary_search_by_key(&"_entities", |(k, _)| *k) {
                    if let Value::Array(arr) = data.remove(entities_idx).1 {
                        return Some(arr);
                    }
                }
                None
            }
            _ => None,
        }
    }

    pub fn to_hash(
        &self,
        selection_items: &[SelectionItem],
        possible_types: &PossibleTypes,
    ) -> u64 {
        let mut hasher = Xxh3::new();
        self.hash_with_requires(&mut hasher, selection_items, possible_types);
        hasher.finish()
    }

    fn hash_with_requires<H: Hasher>(
        &self,
        state: &mut H,
        selection_items: &[SelectionItem],
        possible_types: &PossibleTypes,
    ) {
        if selection_items.is_empty() {
            self.hash(state);
            return;
        }

        match self {
            Value::Object(obj) => {
                Value::hash_object_with_requires(state, obj, selection_items, possible_types);
            }
            Value::Array(arr) => {
                for item in arr {
                    item.hash_with_requires(state, selection_items, possible_types);
                }
            }
            _ => {
                self.hash(state);
            }
        }
    }

    fn hash_object_with_requires<H: Hasher>(
        state: &mut H,
        obj: &[(&'a str, Value<'a>)],
        selection_items: &[SelectionItem],
        possible_types: &PossibleTypes,
    ) {
        for item in selection_items {
            match item {
                SelectionItem::Field(field_selection) => {
                    let field_name = &field_selection.name;
                    if let Ok(idx) = obj.binary_search_by_key(&field_name.as_str(), |(k, _)| k) {
                        let (key, value) = &obj[idx];
                        key.hash(state);
                        value.hash_with_requires(
                            state,
                            &field_selection.selections.items,
                            possible_types,
                        );
                    }
                }
                SelectionItem::InlineFragment(inline_fragment) => {
                    let type_condition = &inline_fragment.type_condition;
                    let type_name = obj
                        .binary_search_by_key(&TYPENAME_FIELD_NAME, |(k, _)| k)
                        .ok()
                        .and_then(|idx| obj[idx].1.as_str())
                        .unwrap_or(type_condition);

                    if possible_types.entity_satisfies_type_condition(type_name, type_condition) {
                        Value::hash_object_with_requires(
                            state,
                            obj,
                            &inline_fragment.selections.items,
                            possible_types,
                        );
                    }
                }
                SelectionItem::FragmentSpread(_) => {
                    unreachable!("Fragment spreads should not exist in FetchNode::requires.")
                }
            }
        }
    }

    pub fn from(json: ValueRef<'a>) -> Value<'a> {
        match json {
            ValueRef::Null => Value::Null,
            ValueRef::Bool(b) => Value::Bool(b),
            ValueRef::String(s) => Value::String(s),
            ValueRef::Number(num) => {
                if let Some(num) = num.as_f64() {
                    return Value::F64(num);
                }

                if let Some(num) = num.as_i64() {
                    return Value::I64(num);
                }

                if let Some(num) = num.as_u64() {
                    return Value::U64(num);
                }

                Value::Null
            }
            ValueRef::Array(arr) => {
                let mut vec = Vec::with_capacity(arr.len());
                vec.extend(arr.iter().map(|v| Value::from(v.as_ref())));
                Value::Array(vec)
            }
            ValueRef::Object(obj) => {
                let mut vec = Vec::with_capacity(obj.len());
                vec.extend(obj.iter().map(|(k, v)| (k, Value::from(v.as_ref()))));
                vec.sort_unstable_by_key(|(k, _)| *k);
                Value::Object(vec)
            }
        }
    }

    pub fn as_object(&self) -> Option<&Vec<(&'a str, Value<'a>)>> {
        match self {
            Value::Object(obj) => Some(obj),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    pub fn is_object(&self) -> bool {
        matches!(self, Value::Object(_))
    }
}

// Our new trait with the desired methods
pub trait ValueRefExt {
    fn to_data<'a>(&'a self) -> Option<ValueRef<'a>>;
    fn to_entities<'a>(&'a self) -> Option<Vec<ValueRef<'a>>>;
}

// Implement our trait for the foreign type
impl ValueRefExt for ValueRef<'_> {
    fn to_data<'a>(&'a self) -> Option<ValueRef<'a>> {
        match self {
            ValueRef::Object(obj) => obj.get(&"data").map(|v| v.as_ref()),
            _ => None,
        }
    }

    fn to_entities<'a>(&'a self) -> Option<Vec<ValueRef<'a>>> {
        match self.to_data().unwrap() {
            ValueRef::Object(obj) => obj.get(&"_entities").and_then(|v| match v.as_ref() {
                ValueRef::Array(arr) => Some(arr.iter().map(|v| v.as_ref()).collect()),
                _ => None,
            }),
            _ => None,
        }
    }
}

impl Display for Value<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Null => write!(f, "null"),
            Value::Bool(b) => write!(f, "{}", b),
            Value::String(s) => write!(f, "\"{}\"", s),
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
            Value::Object(obj) => {
                write!(f, "{{")?;
                for (i, (k, v)) in obj.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "\"{}\": {}", k, v)?;
                }
                write!(f, "}}")
            }
        }
    }
}

struct ValueVisitor<'a> {
    // We need a marker to hold the lifetime 'a.
    _marker: std::marker::PhantomData<&'a ()>,
}

impl<'de> de::Deserialize<'de> for Value<'de> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(ValueVisitor {
            _marker: std::marker::PhantomData,
        })
    }
}

impl<'de> Visitor<'de> for ValueVisitor<'de> {
    type Value = Value<'de>;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("any valid JSON value")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(Value::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(Value::I64(value))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(Value::U64(value))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E> {
        Ok(Value::F64(value))
    }

    // This is the zero-copy part. We borrow the string slice directly from the input.
    fn visit_borrowed_str<E>(self, value: &'de str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(Value::String(value))
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(Value::Null)
    }

    // For arrays, Serde recursively calls `deserialize` for the inner type.
    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut elements = Vec::with_capacity(seq.size_hint().unwrap_or(0));
        while let Some(elem) = seq.next_element()? {
            elements.push(elem);
        }
        Ok(Value::Array(elements))
    }

    // For objects, the same happens for keys and values.
    fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
    where
        M: MapAccess<'de>,
    {
        let mut entries = Vec::with_capacity(map.size_hint().unwrap_or(0));
        while let Some((key, value)) = map.next_entry()? {
            entries.push((key, value));
        }
        // IMPORTANT: We keep the sort for binary search compatibility.
        entries.sort_unstable_by_key(|(k, _)| *k);
        Ok(Value::Object(entries))
    }
}

impl serde::Serialize for Value<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Value::Null => serializer.serialize_unit(),
            Value::Bool(b) => serializer.serialize_bool(*b),
            Value::I64(n) => serializer.serialize_i64(*n),
            Value::U64(n) => serializer.serialize_u64(*n),
            Value::F64(n) => serializer.serialize_f64(*n),
            Value::String(s) => serializer.serialize_str(s),
            Value::Array(arr) => {
                let mut seq = serializer.serialize_seq(Some(arr.len()))?;
                for v in arr {
                    seq.serialize_element(v)?;
                }
                seq.end()
            }
            Value::Object(obj) => {
                let mut map = serializer.serialize_map(Some(obj.len()))?;
                for (k, v) in obj {
                    map.serialize_entry(k, v)?;
                }
                map.end()
            }
        }
    }
}
