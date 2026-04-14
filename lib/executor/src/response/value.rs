use core::fmt;
use hive_router_query_planner::ast::selection_item::SelectionItem;
use serde::{
    de::{self, Deserializer, MapAccess, SeqAccess, Visitor},
    ser::{SerializeMap, SerializeSeq},
};
use sonic_rs::{JsonNumberTrait, Object, ValueRef};
use std::{
    borrow::Cow,
    fmt::Display,
    hash::{Hash, Hasher},
};
use xxhash_rust::xxh3::Xxh3;

use crate::{introspection::schema::PossibleTypes, utils::consts::TYPENAME_FIELD_NAME};

#[derive(Debug, Clone, Default)]
pub enum Value<'a> {
    #[default]
    Null,
    F64(f64),
    I64(i64),
    U64(u64),
    Bool(bool),
    String(Cow<'a, str>),
    Array(Vec<Value<'a>>),
    Object(ValueObject<'a>),
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
    pub fn take_entities(&mut self) -> Option<Vec<Value<'a>>> {
        self.take_entities_by_key("_entities")
    }

    pub fn take_entities_by_key(&mut self, key: &str) -> Option<Vec<Value<'a>>> {
        match self {
            Value::Object(data) => {
                if let Some(Value::Array(arr)) = data.take(key) {
                    return Some(arr);
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
                obj.hash_object_with_requires(state, selection_items, possible_types)
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

    pub fn from(json: ValueRef<'a>) -> Value<'a> {
        match json {
            ValueRef::Null => Value::Null,
            ValueRef::Bool(b) => Value::Bool(b),
            ValueRef::String(s) => Value::String(s.into()),
            ValueRef::Number(num) => {
                if let Some(num) = num.as_u64() {
                    return Value::U64(num);
                }

                if let Some(num) = num.as_i64() {
                    return Value::I64(num);
                }

                // All integers are floats, so to prevent to add
                // extra dots for integer numbers, we check for f64 last.
                if let Some(num) = num.as_f64() {
                    return Value::F64(num);
                }

                Value::Null
            }
            ValueRef::Array(arr) => {
                let mut vec = Vec::with_capacity(arr.len());
                vec.extend(arr.iter().map(|v| Value::from(v.as_ref())));
                Value::Array(vec)
            }
            ValueRef::Object(obj) => Value::Object(obj.into()),
        }
    }

    pub fn as_object(&self) -> Option<&ValueObject<'a>> {
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
            Value::Object(obj) => obj.fmt(f),
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
        Ok(Value::String(value.into()))
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(Value::String(v.to_owned().into()))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(Value::String(value.into()))
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
        Ok(Value::Object(entries.into()))
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
                for (k, v) in obj.iter() {
                    map.serialize_entry(k, &v)?;
                }
                map.end()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Value;
    use serde::Deserialize;
    use std::borrow::Cow;

    #[test]
    fn deserializes_escaped_string_as_owned() {
        let bytes = br#"{"message": "hello\nworld"}"#;
        let mut deserializer = sonic_rs::Deserializer::from_slice(bytes);

        let value = Value::deserialize(&mut deserializer).unwrap();

        let obj = match value {
            Value::Object(obj) => obj,
            _ => panic!("Expected Value::Object"),
        };

        let message_value = &obj.get("message").unwrap();

        match message_value {
            Value::String(value) => {
                assert_eq!(value, "hello\nworld");
                assert!(
                    matches!(value, Cow::Owned(_)),
                    "Expected Cow::Owned for escaped string"
                );
            }
            _ => panic!("Expected Value::String"),
        }
    }

    #[test]
    fn deserializes_simple_string_as_borrowed() {
        let bytes = br#"{"message": "hello world"}"#;
        let mut deserializer = sonic_rs::Deserializer::from_slice(bytes);
        let value = Value::deserialize(&mut deserializer).unwrap();

        let obj = match value {
            Value::Object(obj) => obj,
            _ => panic!("Expected Value::Object"),
        };

        let message_value = &obj.get("message").unwrap();

        match message_value {
            Value::String(value) => {
                assert_eq!(value, "hello world");
                assert!(
                    matches!(value, Cow::Borrowed(_)),
                    "Expected Cow::Borrowed for simple string"
                );
            }
            _ => panic!("Expected Value::String"),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ValueObject<'a> {
    pub(crate) entries: Vec<(&'a str, Value<'a>)>,
}

impl<'a> From<&'a Object> for ValueObject<'a> {
    fn from(obj: &'a Object) -> Self {
        let mut entries = Vec::with_capacity(obj.len());
        entries.extend(obj.iter().map(|(k, v)| (k, Value::from(v.as_ref()))));
        entries.into()
    }
}

impl<'a> Hash for ValueObject<'a> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.entries.hash(state)
    }
}

impl<'a> Display for ValueObject<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{{")?;
        for (i, (k, v)) in self.entries.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "\"{}\": {}", k, v)?;
        }
        write!(f, "}}")
    }
}

impl<'a> ValueObject<'a> {
    pub fn get(&self, key: &str) -> Option<&Value<'a>> {
        self.find_index(key).map(|idx| &self.entries[idx].1)
    }
    pub fn get_mut(&mut self, key: &str) -> Option<&mut Value<'a>> {
        self.find_index(key).map(|idx| &mut self.entries[idx].1)
    }
    pub fn take(&mut self, key: &str) -> Option<Value<'a>> {
        self.find_index(key).map(|idx| self.entries.remove(idx).1)
    }
    pub fn entry(&mut self, key: &str) -> Option<&mut (&'a str, Value<'a>)> {
        self.find_index(key).and_then(|i| self.entries.get_mut(i))
    }
    pub fn type_name(&self) -> Option<&str> {
        self.get(TYPENAME_FIELD_NAME).and_then(|v| v.as_str())
    }
    fn find_index(&self, key: &str) -> Option<usize> {
        self.entries.binary_search_by_key(&key, |(k, _)| *k).ok()
    }
    pub fn hash_object_with_requires<H: Hasher>(
        &self,
        state: &mut H,
        selection_items: &[SelectionItem],
        possible_types: &PossibleTypes,
    ) {
        for item in selection_items {
            match item {
                SelectionItem::Field(field_selection) => {
                    let field_name = &field_selection.name;
                    if let Some(value) = self.get(field_name) {
                        field_name.hash(state);
                        value.hash_with_requires(
                            state,
                            &field_selection.selections.items,
                            possible_types,
                        );
                    }
                }
                SelectionItem::InlineFragment(inline_fragment) => {
                    let type_condition = &inline_fragment.type_condition;
                    let type_name = self.type_name().unwrap_or(type_condition);

                    if possible_types.entity_satisfies_type_condition(type_name, type_condition) {
                        self.hash_object_with_requires(
                            state,
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
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    pub fn clear(&mut self) {
        self.entries.clear();
    }
    pub fn extend<I: IntoIterator<Item = (&'a str, Value<'a>)>>(&mut self, iter: I) {
        self.entries.extend(iter);
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn with_capacity(capacity: usize) -> Self {
        ValueObject {
            entries: Vec::with_capacity(capacity),
        }
    }
    pub fn push(&mut self, entry: (&'a str, Value<'a>)) {
        self.entries.push(entry);
    }

    pub fn iter(&self) -> impl Iterator<Item = (&'a str, &Value<'a>)> {
        self.entries.iter().map(|(k, v)| (*k, v))
    }
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&'a str, &mut Value<'a>)> {
        self.entries.iter_mut().map(|(k, v)| (*k, v))
    }
}

impl<'a> IntoIterator for ValueObject<'a> {
    type Item = (&'a str, Value<'a>);
    type IntoIter = std::vec::IntoIter<(&'a str, Value<'a>)>;

    fn into_iter(self) -> Self::IntoIter {
        self.entries.into_iter()
    }
}

impl<'a> From<Vec<(&'a str, Value<'a>)>> for ValueObject<'a> {
    fn from(mut entries: Vec<(&'a str, Value<'a>)>) -> Self {
        entries.sort_unstable_by_key(|(k, _)| *k);
        ValueObject { entries }
    }
}

impl<'a> From<ValueObject<'a>> for Value<'a> {
    fn from(obj: ValueObject<'a>) -> Self {
        Value::Object(obj)
    }
}
