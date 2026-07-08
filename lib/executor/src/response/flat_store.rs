use std::borrow::Cow;
use std::ops::Range;

/// Compact identifier for a value stored in the flat store.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FlatValueId(pub u32);

impl FlatValueId {
    pub fn new(raw: u32) -> Self {
        FlatValueId(raw)
    }

    pub fn raw(&self) -> u32 {
        self.0
    }
}

/// Compact identifier for a response key string in the key table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ResponseKeyId(pub u32);

/// A leaf value in the flat store.
#[derive(Debug, Clone)]
pub enum FlatValue<'a> {
    Null,
    Bool(bool),
    I64(i64),
    U64(u64),
    F64(f64),
    String(Cow<'a, str>),
    RawJson(Cow<'a, str>),
    Object {
        fields: Range<u32>,
    },
    List {
        items: Range<u32>,
    },
    Missing,
    Inaccessible,
}

/// One field of a flat object.
#[derive(Debug, Clone)]
pub struct FlatObjectField {
    pub response_key: ResponseKeyId,
    pub value: FlatValueId,
}

/// Immutable string table mapping `ResponseKeyId` to a response key string
/// and its pre-serialized JSON form `"key":`.
#[derive(Debug, Clone, Default)]
pub struct ResponseKeys {
    keys: Vec<Box<str>>,
    serialized_keys: Vec<Box<[u8]>>,
}

impl ResponseKeys {
    pub fn intern(&mut self, key: &str) -> ResponseKeyId {
        if let Some(pos) = self.keys.iter().position(|k| k.as_ref() == key) {
            return ResponseKeyId(pos as u32);
        }
        let id = ResponseKeyId(self.keys.len() as u32);
        let mut serialized = Vec::with_capacity(key.len() + 4);
        serialized.push(b'"');
        serialized.extend_from_slice(key.as_bytes());
        serialized.push(b'"');
        serialized.push(b':');
        self.keys.push(key.into());
        self.serialized_keys.push(serialized.into_boxed_slice());
        id
    }

    pub fn key(&self, id: ResponseKeyId) -> &str {
        &self.keys[id.0 as usize]
    }

    pub fn serialized_json_key(&self, id: ResponseKeyId) -> &[u8] {
        &self.serialized_keys[id.0 as usize]
    }

    pub fn len(&self) -> usize {
        self.keys.len()
    }
}

/// The normalized flat response store.
///
/// All response data is stored as a linear array of values. Objects and lists
/// store ranges into the object-fields and list-items arrays.
#[derive(Debug, Clone, Default)]
pub struct FlatResponseStore<'a> {
    values: Vec<FlatValue<'a>>,
    object_fields: Vec<FlatObjectField>,
    list_items: Vec<FlatValueId>,
    /// Bytes that owned strings/raw-json borrow from.
    retained_bytes: Vec<bytes::Bytes>,
}

/// Builder for assembling an object's fields directly into the store.
pub struct ObjectFieldsBuilder {
    start: u32,
    len: u32,
}

impl<'a> FlatResponseStore<'a> {
    pub fn new() -> Self {
        Self::default()
    }

    // -- direct object fields builder --

    /// Begin building an object's fields directly in the store.
    pub fn begin_object_fields(&mut self, capacity: usize) -> ObjectFieldsBuilder {
        self.object_fields.reserve(capacity);
        let start = self.object_fields.len() as u32;
        ObjectFieldsBuilder { start, len: 0 }
    }

    pub fn push_object_field(&mut self, builder: &mut ObjectFieldsBuilder, field: FlatObjectField) {
        self.object_fields.push(field);
        builder.len += 1;
    }

    pub fn finish_object_fields(&mut self, builder: ObjectFieldsBuilder) -> FlatValueId {
        self.push_value(FlatValue::Object {
            fields: builder.start..builder.start + builder.len,
        })
    }

    // -- allocation --

    pub fn alloc_null(&mut self) -> FlatValueId {
        self.push_value(FlatValue::Null)
    }

    pub fn alloc_bool(&mut self, value: bool) -> FlatValueId {
        self.push_value(FlatValue::Bool(value))
    }

    pub fn alloc_i64(&mut self, value: i64) -> FlatValueId {
        self.push_value(FlatValue::I64(value))
    }

    pub fn alloc_u64(&mut self, value: u64) -> FlatValueId {
        self.push_value(FlatValue::U64(value))
    }

    pub fn alloc_f64(&mut self, value: f64) -> FlatValueId {
        self.push_value(FlatValue::F64(value))
    }

    pub fn alloc_string(&mut self, value: Cow<'a, str>) -> FlatValueId {
        self.push_value(FlatValue::String(value))
    }

    pub fn alloc_raw_json(&mut self, value: Cow<'a, str>) -> FlatValueId {
        self.push_value(FlatValue::RawJson(value))
    }

    pub fn alloc_missing(&mut self) -> FlatValueId {
        self.push_value(FlatValue::Missing)
    }

    pub fn alloc_inaccessible(&mut self) -> FlatValueId {
        self.push_value(FlatValue::Inaccessible)
    }

    pub fn alloc_object(&mut self, fields: Vec<FlatObjectField>) -> FlatValueId {
        let start = self.object_fields.len() as u32;
        let end = start + fields.len() as u32;
        self.object_fields.extend(fields);
        self.push_value(FlatValue::Object {
            fields: start..end,
        })
    }

    pub fn alloc_list_from_items(&mut self, mut items: Vec<FlatValueId>) -> FlatValueId {
        let start = self.list_items.len() as u32;
        let end = start + items.len() as u32;
        self.list_items.append(&mut items);
        self.push_value(FlatValue::List {
            items: start..end,
        })
    }

    /// Reserve a list id range and return a list builder.
    pub fn reserve_list(&mut self, capacity: usize) -> FlatListBuilder {
        let start = self.list_items.len() as u32;
        FlatListBuilder {
            start,
            items: Vec::with_capacity(capacity),
        }
    }

    /// Finalize a list builder into a store value id.
    pub fn finish_list(&mut self, builder: FlatListBuilder) -> FlatValueId {
        let end = builder.start + builder.items.len() as u32;
        self.list_items.extend(builder.items);
        self.push_value(FlatValue::List {
            items: builder.start..end,
        })
    }

    pub fn retain_bytes(&mut self, bytes: bytes::Bytes) {
        self.retained_bytes.push(bytes);
    }

    // -- access --

    pub fn value(&self, id: FlatValueId) -> &FlatValue<'a> {
        &self.values[id.0 as usize]
    }

    pub fn value_mut(&mut self, id: FlatValueId) -> &mut FlatValue<'a> {
        &mut self.values[id.0 as usize]
    }

    pub fn object_fields(&self, range: &Range<u32>) -> &[FlatObjectField] {
        &self.object_fields[range.start as usize..range.end as usize]
    }

    pub fn list_items(&self, range: &Range<u32>) -> &[FlatValueId] {
        &self.list_items[range.start as usize..range.end as usize]
    }

    pub fn set_value(&mut self, id: FlatValueId, value: FlatValue<'a>) {
        self.values[id.0 as usize] = value;
    }

    // -- merge helpers --

    pub fn merge_value(&mut self, target_id: FlatValueId, source_id: FlatValueId) {
        let source_val = self.value(source_id).clone();
        let target_clone = self.value(target_id).clone();

        match (target_clone, source_val) {
            (FlatValue::Object { fields: target_range }, FlatValue::Object { fields: source_range }) => {
                let source_fields =
                    self.object_fields[source_range.start as usize..source_range.end as usize].to_vec();
                let target_fields =
                    &self.object_fields[target_range.start as usize..target_range.end as usize];
                let mut new_fields = target_fields.to_vec();
                for sf in source_fields {
                    if let Some(existing) =
                        new_fields.iter_mut().find(|f| f.response_key == sf.response_key)
                    {
                        self.merge_value(existing.value, sf.value);
                    } else {
                        new_fields.push(sf);
                    }
                }
                let start = self.object_fields.len() as u32;
                let end = start + new_fields.len() as u32;
                self.object_fields.extend(new_fields);
                self.values[target_id.0 as usize] = FlatValue::Object { fields: start..end };
            }
            (FlatValue::List { items: target_range }, FlatValue::List { items: source_range }) => {
                let source_items =
                    self.list_items[source_range.start as usize..source_range.end as usize].to_vec();
                let target_items =
                    &self.list_items[target_range.start as usize..target_range.end as usize];
                let mut new_items = target_items.to_vec();
                for (ti, si) in new_items.iter_mut().zip(source_items) {
                    self.merge_value(*ti, si);
                }
                let start = self.list_items.len() as u32;
                let end = start + new_items.len() as u32;
                self.list_items.extend(new_items);
                self.values[target_id.0 as usize] = FlatValue::List { items: start..end };
            }
            (_, FlatValue::Null) => { /* noop */ }
            (_, source) => {
                self.values[target_id.0 as usize] = source;
            }
        }
    }

    pub fn insert_empty_fields(
        &mut self,
        object_id: FlatValueId,
        fields: impl IntoIterator<Item = FlatObjectField>,
    ) {
        if let FlatValue::Object { fields: target_range } = self.value(object_id).clone() {
            let target_fields = self.object_fields[target_range.start as usize..target_range.end as usize]
                .to_vec();
            let mut new_fields = target_fields;
            for field in fields {
                if !new_fields.iter().any(|f| f.response_key == field.response_key) {
                    new_fields.push(field);
                }
            }
            let start = self.object_fields.len() as u32;
            let end = start + new_fields.len() as u32;
            self.object_fields.extend(new_fields);
            self.values[object_id.0 as usize] = FlatValue::Object { fields: start..end };
        }
    }

    pub fn clear(&mut self) {
        self.values.clear();
        self.object_fields.clear();
        self.list_items.clear();
        self.retained_bytes.clear();
    }

    fn push_value(&mut self, value: FlatValue<'a>) -> FlatValueId {
        let id = FlatValueId(self.values.len() as u32);
        self.values.push(value);
        id
    }
}

pub struct FlatListBuilder {
    start: u32,
    items: Vec<FlatValueId>,
}

impl FlatListBuilder {
    pub fn push(&mut self, id: FlatValueId) {
        self.items.push(id);
    }
}

// -- Serializer --

use bytes::BufMut;

use crate::json_writer::{write_and_escape_string, write_f64, write_i64, write_u64};
use crate::utils::consts::{
    CLOSE_BRACE, CLOSE_BRACKET, COMMA, FALSE, NULL, OPEN_BRACE, OPEN_BRACKET, TRUE,
};

impl<'a> FlatResponseStore<'a> {
    /// Serialize a value into a JSON buffer using the provided key table.
    pub fn serialize_value(
        &self,
        id: FlatValueId,
        keys: &ResponseKeys,
        buffer: &mut Vec<u8>,
    ) {
        self.serialize_value_impl(id, keys, buffer);
    }

    fn serialize_value_impl(
        &self,
        id: FlatValueId,
        keys: &ResponseKeys,
        buffer: &mut Vec<u8>,
    ) {
        match self.value(id) {
            FlatValue::Null | FlatValue::Missing | FlatValue::Inaccessible => buffer.put(NULL),
            FlatValue::Bool(true) => buffer.put(TRUE),
            FlatValue::Bool(false) => buffer.put(FALSE),
            FlatValue::I64(num) => write_i64(buffer, *num),
            FlatValue::U64(num) => write_u64(buffer, *num),
            FlatValue::F64(num) => write_f64(buffer, *num),
            FlatValue::String(s) => write_and_escape_string(buffer, s),
            FlatValue::RawJson(raw) => buffer.put_slice(raw.as_bytes()),
            FlatValue::Object { fields } => self.serialize_object(fields, keys, buffer),
            FlatValue::List { items } => self.serialize_list(items, keys, buffer),
        }
    }

    fn serialize_object(
        &self,
        fields: &Range<u32>,
        keys: &ResponseKeys,
        buffer: &mut Vec<u8>,
    ) {
        buffer.put(OPEN_BRACE);
        let field_slice = self.object_fields(fields);
        let mut first = true;
        for field in field_slice {
            if !first {
                buffer.put(COMMA);
            }
            first = false;
            buffer.put_slice(keys.serialized_json_key(field.response_key));
            self.serialize_value_impl(field.value, keys, buffer);
        }
        buffer.put(CLOSE_BRACE);
    }

    fn serialize_list(
        &self,
        items: &Range<u32>,
        keys: &ResponseKeys,
        buffer: &mut Vec<u8>,
    ) {
        buffer.put(OPEN_BRACKET);
        let item_slice = self.list_items(items);
        let mut first = true;
        for &item_id in item_slice {
            if !first {
                buffer.put(COMMA);
            }
            first = false;
            self.serialize_value_impl(item_id, keys, buffer);
        }
        buffer.put(CLOSE_BRACKET);
    }
}

/// Serialize the full response data root into a JSON buffer.
///
/// Writes `{"data": <value>}` into the buffer.
pub fn serialize_store_data(
    store: &FlatResponseStore<'_>,
    keys: &ResponseKeys,
    root: FlatValueId,
    buffer: &mut Vec<u8>,
) {
    buffer.put(OPEN_BRACE);
    buffer.put_slice(b"\"data\":");
    store.serialize_value(root, keys, buffer);
    buffer.put(CLOSE_BRACE);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alloc_and_read_scalar() {
        let mut store = FlatResponseStore::new();
        let id = store.alloc_string(Cow::Borrowed("hello"));
        assert!(matches!(store.value(id), FlatValue::String(s) if s == "hello"));
    }

    #[test]
    fn alloc_object_and_iterate_fields() {
        let mut store = FlatResponseStore::new();
        let mut keys = ResponseKeys::default();
        let k_name = keys.intern("name");
        let v_name = store.alloc_string(Cow::Borrowed("Alice"));
        let obj = store.alloc_object(vec![FlatObjectField {
            response_key: k_name,
            value: v_name,
        }]);

        let FlatValue::Object { fields } = store.value(obj) else {
            panic!("expected object");
        };
        let field_slice = store.object_fields(fields);
        assert_eq!(field_slice.len(), 1);
        assert_eq!(field_slice[0].response_key, k_name);
        assert!(matches!(store.value(field_slice[0].value), FlatValue::String(s) if s == "Alice"));
    }

    #[test]
    fn alloc_list() {
        let mut store = FlatResponseStore::new();
        let a = store.alloc_i64(1);
        let b = store.alloc_i64(2);
        let list = store.alloc_list_from_items(vec![a, b]);

        let FlatValue::List { items } = store.value(list) else {
            panic!("expected list");
        };
        let item_slice = store.list_items(items);
        assert_eq!(item_slice.len(), 2);
        assert!(matches!(store.value(item_slice[0]), FlatValue::I64(1)));
        assert!(matches!(store.value(item_slice[1]), FlatValue::I64(2)));
    }

    #[test]
    fn merge_objects() {
        let mut store = FlatResponseStore::new();
        let k1 = ResponseKeyId(0);
        let k2 = ResponseKeyId(1);

        let v1 = store.alloc_string(Cow::Borrowed("a"));
        let obj1 = store.alloc_object(vec![FlatObjectField {
            response_key: k1,
            value: v1,
        }]);

        let v2 = store.alloc_string(Cow::Borrowed("b"));
        let obj2 = store.alloc_object(vec![FlatObjectField {
            response_key: k2,
            value: v2,
        }]);

        store.merge_value(obj1, obj2);

        let FlatValue::Object { fields } = store.value(obj1) else {
            panic!("expected object");
        };
        let field_slice = store.object_fields(fields);
        assert_eq!(field_slice.len(), 2);
    }

    #[test]
    fn merge_null_is_noop() {
        let mut store = FlatResponseStore::new();
        let v = store.alloc_i64(42);
        let null_id = store.alloc_null();
        store.merge_value(v, null_id);
        assert!(matches!(store.value(v), FlatValue::I64(42)));
    }

    #[test]
    fn serialize_scalars() {
        let mut store = FlatResponseStore::new();
        let keys = ResponseKeys::default();

        let null_id = store.alloc_null();
        let bool_id = store.alloc_bool(true);
        let i64_id = store.alloc_i64(-42);
        let u64_id = store.alloc_u64(100);
        let f64_id = store.alloc_f64(3.14);
        let str_id = store.alloc_string(Cow::Borrowed("hello"));

        let mut buf = Vec::new();
        store.serialize_value(null_id, &keys, &mut buf);
        assert_eq!(std::str::from_utf8(&buf).unwrap(), "null");

        buf.clear();
        store.serialize_value(bool_id, &keys, &mut buf);
        assert_eq!(std::str::from_utf8(&buf).unwrap(), "true");

        buf.clear();
        store.serialize_value(i64_id, &keys, &mut buf);
        assert_eq!(std::str::from_utf8(&buf).unwrap(), "-42");

        buf.clear();
        store.serialize_value(u64_id, &keys, &mut buf);
        assert_eq!(std::str::from_utf8(&buf).unwrap(), "100");

        buf.clear();
        store.serialize_value(f64_id, &keys, &mut buf);
        // 3.14 is a float
        assert!(std::str::from_utf8(&buf).unwrap().starts_with("3.14"));

        buf.clear();
        store.serialize_value(str_id, &keys, &mut buf);
        assert_eq!(std::str::from_utf8(&buf).unwrap(), "\"hello\"");
    }

    #[test]
    fn serialize_object() {
        let mut store = FlatResponseStore::new();
        let mut keys = ResponseKeys::default();
        let k_name = keys.intern("name");
        let k_age = keys.intern("age");

        let v_name = store.alloc_string(Cow::Borrowed("Alice"));
        let v_age = store.alloc_i64(30);
        let obj = store.alloc_object(vec![
            FlatObjectField {
                response_key: k_name,
                value: v_name,
            },
            FlatObjectField {
                response_key: k_age,
                value: v_age,
            },
        ]);

        let mut buf = Vec::new();
        store.serialize_value(obj, &keys, &mut buf);
        let json = std::str::from_utf8(&buf).unwrap();
        assert_eq!(json, "{\"name\":\"Alice\",\"age\":30}");
    }

    #[test]
    fn serialize_list() {
        let mut store = FlatResponseStore::new();
        let keys = ResponseKeys::default();

        let a = store.alloc_i64(1);
        let b = store.alloc_i64(2);
        let list = store.alloc_list_from_items(vec![a, b]);

        let mut buf = Vec::new();
        store.serialize_value(list, &keys, &mut buf);
        assert_eq!(std::str::from_utf8(&buf).unwrap(), "[1,2]");
    }

    #[test]
    fn serialize_store_data_wraps_in_data_key() {
        let mut store = FlatResponseStore::new();
        let mut keys = ResponseKeys::default();
        let k_x = keys.intern("x");
        let v_x = store.alloc_i64(1);
        let obj = store.alloc_object(vec![FlatObjectField {
            response_key: k_x,
            value: v_x,
        }]);

        let mut buf = Vec::new();
        serialize_store_data(&store, &keys, obj, &mut buf);
        assert_eq!(std::str::from_utf8(&buf).unwrap(), "{\"data\":{\"x\":1}}");
    }

    #[test]
    fn response_keys_intern_and_serialize() {
        let mut keys = ResponseKeys::default();
        let id = keys.intern("name");
        assert_eq!(keys.key(id), "name");
        assert_eq!(keys.serialized_json_key(id), b"\"name\":");
    }

    #[test]
    fn response_keys_deduplicate() {
        let mut keys = ResponseKeys::default();
        let id1 = keys.intern("name");
        let id2 = keys.intern("name");
        assert_eq!(id1, id2);
        assert_eq!(keys.len(), 1);
    }
}
