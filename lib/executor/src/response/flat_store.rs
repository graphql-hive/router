use std::borrow::Cow;
use std::hash::{Hash, Hasher};
use std::ops::Range;

use ahash::AHashMap as HashMap;
use bytes::BufMut;
use hive_router_query_planner::ast::selection_item::SelectionItem;
use hive_router_query_planner::planner::plan_nodes::FlattenNodePathSegment;
use lasso2::{Key, Rodeo};
use xxhash_rust::xxh3::Xxh3;

use crate::introspection::schema::PossibleTypes;
use crate::utils::consts::{
    CLOSE_BRACE, CLOSE_BRACKET, COLON, COMMA, FALSE, NULL, OPEN_BRACE, OPEN_BRACKET, QUOTE, TRUE,
    TYPENAME_FIELD_NAME, TYPENAME_JSON_FIELD,
};

/// Compact identifier for a value stored in the flat store.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ResponseKeyId(pub u32);

unsafe impl Key for ResponseKeyId {
    #[inline]
    fn into_usize(self) -> usize {
        self.0 as usize
    }

    #[inline]
    fn try_from_usize(int: usize) -> Option<Self> {
        u32::try_from(int).ok().map(ResponseKeyId)
    }
}

type ResponseKeyInterner = Rodeo<ResponseKeyId, ahash::RandomState>;

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
    Object { fields: Range<u32> },
    List { items: Range<u32> },
    Missing,
    Inaccessible,
}

/// One field of a flat object.
#[derive(Debug, Clone)]
pub struct FlatObjectField {
    pub response_key: ResponseKeyId,
    pub value: FlatValueId,
    pub output_key: Option<ResponseKeyId>,
    pub output_position: Option<u16>,
    pub is_non_null: bool,
}

impl FlatObjectField {
    pub fn new(response_key: ResponseKeyId, value: FlatValueId) -> Self {
        Self {
            response_key,
            value,
            output_key: None,
            output_position: None,
            is_non_null: false,
        }
    }

    pub fn with_output(
        response_key: ResponseKeyId,
        value: FlatValueId,
        output_key: Option<ResponseKeyId>,
        output_position: Option<u16>,
        is_non_null: bool,
    ) -> Self {
        Self {
            response_key,
            value,
            output_key,
            output_position,
            is_non_null,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct FlatStoreAppendMap {
    value_offset: u32,
}

impl FlatStoreAppendMap {
    pub fn value(&self, id: FlatValueId) -> FlatValueId {
        FlatValueId(id.0 + self.value_offset)
    }
}

/// Immutable string table mapping `ResponseKeyId` to a response key string
/// and its pre-serialized JSON form `"key":`.
#[derive(Debug, Clone)]
pub struct ResponseKeys {
    interner: ResponseKeyInterner,
    serialized_keys: Vec<Box<[u8]>>,
}

impl Default for ResponseKeys {
    fn default() -> Self {
        Self {
            interner: ResponseKeyInterner::with_hasher(ahash::RandomState::new()),
            serialized_keys: Vec::new(),
        }
    }
}

impl ResponseKeys {
    pub fn intern(&mut self, key: &str) -> ResponseKeyId {
        if let Some(id) = self.interner.get(key) {
            return id;
        }
        let id = self.interner.get_or_intern(key);
        debug_assert_eq!(id.0 as usize, self.serialized_keys.len());
        let mut serialized = Vec::with_capacity(key.len() + 4);
        serialized.push(b'"');
        serialized.extend_from_slice(key.as_bytes());
        serialized.push(b'"');
        serialized.push(b':');
        self.serialized_keys.push(serialized.into_boxed_slice());
        id
    }

    /// Look up a key id without mutating. Returns None if not found.
    pub fn get_key_id(&self, key: &str) -> Option<ResponseKeyId> {
        self.interner.get(key)
    }

    pub fn key(&self, id: ResponseKeyId) -> &str {
        self.interner.resolve(&id)
    }

    pub fn serialized_json_key(&self, id: ResponseKeyId) -> &[u8] {
        &self.serialized_keys[id.0 as usize]
    }

    pub fn len(&self) -> usize {
        self.serialized_keys.len()
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

    pub fn with_capacity(
        values: usize,
        object_fields: usize,
        list_items: usize,
        retained_bytes: usize,
    ) -> Self {
        Self {
            values: Vec::with_capacity(values),
            object_fields: Vec::with_capacity(object_fields),
            list_items: Vec::with_capacity(list_items),
            retained_bytes: Vec::with_capacity(retained_bytes),
        }
    }

    pub fn with_response_size_hint(response_size: usize) -> Self {
        let values = (response_size / 64).clamp(16, 512);
        let object_fields = (response_size / 96).clamp(8, 512);
        let list_items = (response_size / 128).clamp(8, 512);

        Self::with_capacity(values, object_fields, list_items, 1)
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
        self.push_value(FlatValue::Object { fields: start..end })
    }

    pub fn alloc_list_from_items(&mut self, mut items: Vec<FlatValueId>) -> FlatValueId {
        let start = self.list_items.len() as u32;
        let end = start + items.len() as u32;
        self.list_items.append(&mut items);
        self.push_value(FlatValue::List { items: start..end })
    }

    /// Reserve a list id range and return a list builder.
    pub fn reserve_list(&mut self, capacity: usize) -> FlatListBuilder {
        FlatListBuilder {
            items: Vec::with_capacity(capacity),
            len: 0,
        }
    }

    pub fn push_list_item(&mut self, builder: &mut FlatListBuilder, id: FlatValueId) {
        builder.items.push(id);
        builder.len += 1;
    }

    pub fn discard_list(&mut self, _builder: FlatListBuilder) {}

    /// Finalize a list builder into a store value id.
    pub fn finish_list(&mut self, builder: FlatListBuilder) -> FlatValueId {
        let start = self.list_items.len() as u32;
        let len = builder.len;
        self.list_items.reserve(builder.items.len());
        self.list_items.extend(builder.items);
        self.push_value(FlatValue::List {
            items: start..start + len,
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

    /// Get a mutable slice of object fields for a range. The caller must ensure
    /// they don't invalidate the range by pushing new fields while borrowing.
    pub fn object_fields_mut(&mut self, range: &Range<u32>) -> &mut [FlatObjectField] {
        &mut self.object_fields[range.start as usize..range.end as usize]
    }

    pub fn list_items(&self, range: &Range<u32>) -> &[FlatValueId] {
        &self.list_items[range.start as usize..range.end as usize]
    }

    /// Find a field index in an object's field range by response key id.
    pub fn find_object_field_by_key_id(
        &self,
        range: &Range<u32>,
        key_id: ResponseKeyId,
    ) -> Option<(usize, FlatValueId)> {
        self.object_fields(range)
            .iter()
            .position(|f| f.response_key == key_id)
            .map(|idx| (idx, self.object_fields(range)[idx].value))
    }

    pub fn set_value(&mut self, id: FlatValueId, value: FlatValue<'a>) {
        self.values[id.0 as usize] = value;
    }

    /// Replace the value at a specific object field.
    pub fn set_object_field_value(
        &mut self,
        object_id: FlatValueId,
        key_id: ResponseKeyId,
        new_value: FlatValueId,
    ) {
        if let FlatValue::Object { fields: range } = self.value(object_id).clone() {
            for sf in self.object_fields_mut(&range) {
                if sf.response_key == key_id {
                    sf.value = new_value;
                    return;
                }
            }
        }
    }

    /// Rename an object field's response key. The new key must already be
    /// interned in ResponseKeys.
    pub fn rename_object_field(
        &mut self,
        object_id: FlatValueId,
        old_key_id: ResponseKeyId,
        new_key_id: ResponseKeyId,
    ) {
        if let FlatValue::Object { fields: range } = self.value(object_id).clone() {
            for sf in self.object_fields_mut(&range) {
                if sf.response_key == old_key_id {
                    sf.response_key = new_key_id;
                    return;
                }
            }
        }
    }

    /// Walk a flat value tree into arrays and find object fields by key name.
    /// Returns a Vec of FlatValueIds for all targets found at the given path.
    /// The path is a sequence of field key names.
    /// Arrays are traversed, objects navigated by field key.
    pub fn resolve_path(
        &self,
        root: FlatValueId,
        keys: &ResponseKeys,
        path_segments: &[String],
    ) -> Vec<FlatValueId> {
        self.resolve_path_impl(root, keys, path_segments, 0)
    }

    fn resolve_path_impl(
        &self,
        current: FlatValueId,
        keys: &ResponseKeys,
        segments: &[String],
        depth: usize,
    ) -> Vec<FlatValueId> {
        if depth >= segments.len() {
            return vec![current];
        }
        let key_name = &segments[depth];
        let Some(key_id) = keys.get_key_id(key_name) else {
            return Vec::new();
        };
        match self.value(current) {
            FlatValue::Object { fields: range } => {
                let mut results = Vec::new();
                for sf in self.object_fields(range) {
                    if sf.response_key == key_id {
                        results.extend(self.resolve_path_impl(sf.value, keys, segments, depth + 1));
                    }
                }
                results
            }
            FlatValue::List { items: range } => {
                let mut results = Vec::new();
                for &item_id in self.list_items(range) {
                    results.extend(self.resolve_path_impl(item_id, keys, segments, depth));
                }
                results
            }
            _ => Vec::new(),
        }
    }

    // -- merge helpers --

    /// Traverse the flat store along a FlattenNodePath and call `callback` for each
    /// leaf entity found (flat equivalent of `traverse_and_callback`).
    pub fn traverse_path<F>(
        &self,
        current: FlatValueId,
        keys: &ResponseKeys,
        path: &[FlattenNodePathSegment],
        depth: usize,
        possible_types: &PossibleTypes,
        callback: &mut F,
    ) where
        F: FnMut(FlatValueId, &[FlattenNodePathSegment], usize),
    {
        if depth >= path.len() {
            match self.value(current) {
                FlatValue::List { items } => {
                    for &item_id in self.list_items(items) {
                        callback(item_id, path, depth);
                    }
                }
                _ => callback(current, path, depth),
            }
            return;
        }

        match &path[depth] {
            FlattenNodePathSegment::List => {
                if let FlatValue::List { items } = self.value(current) {
                    let next_depth = depth + 1;
                    for &item_id in self.list_items(items) {
                        self.traverse_path(
                            item_id,
                            keys,
                            path,
                            next_depth,
                            possible_types,
                            callback,
                        );
                    }
                }
            }
            FlattenNodePathSegment::Field(field_name) => {
                let Some(key_id) = keys.get_key_id(field_name.as_str()) else {
                    return;
                };
                if let FlatValue::Object { fields } = self.value(current) {
                    let next_depth = depth + 1;
                    for sf in self.object_fields(fields) {
                        if sf.response_key == key_id {
                            self.traverse_path(
                                sf.value,
                                keys,
                                path,
                                next_depth,
                                possible_types,
                                callback,
                            );
                        }
                    }
                }
            }
            FlattenNodePathSegment::TypeCondition(type_conditions) => match self.value(current) {
                FlatValue::Object { fields } => {
                    let type_name = self.object_fields(fields).iter().find_map(|sf| {
                        (keys.key(sf.response_key) == "__typename").then(|| {
                            if let FlatValue::String(s) = self.value(sf.value) {
                                Some(s.as_ref())
                            } else {
                                None
                            }
                        })?
                    });
                    let matches = type_name.is_none_or(|type_name| {
                        type_conditions.iter().any(|cond| {
                            possible_types.entity_satisfies_type_condition(type_name, cond)
                        })
                    });
                    if matches {
                        let next_depth = depth + 1;
                        self.traverse_path(
                            current,
                            keys,
                            path,
                            next_depth,
                            possible_types,
                            callback,
                        );
                    }
                }
                FlatValue::List { items } => {
                    for &item_id in self.list_items(items) {
                        self.traverse_path(item_id, keys, path, depth, possible_types, callback);
                    }
                }
                _ => {}
            },
        }
    }

    /// Get the __typename of a flat object, if present.
    pub fn get_typename(&self, object_id: FlatValueId, keys: &ResponseKeys) -> Option<&str> {
        if let FlatValue::Object { fields } = self.value(object_id) {
            let tk_id = keys.get_key_id("__typename")?;
            for sf in self.object_fields(fields) {
                if sf.response_key == tk_id {
                    if let FlatValue::String(s) = self.value(sf.value) {
                        return Some(s.as_ref());
                    }
                }
            }
        }
        None
    }

    /// Serialize the selected fields of a flat entity into a JSON buffer.
    /// Used to build `_entities` representation variables.
    pub fn serialize_entity_requires_to_buffer(
        &self,
        entity_id: FlatValueId,
        keys: &ResponseKeys,
        buffer: &mut Vec<u8>,
        first: bool,
    ) -> bool {
        if let FlatValue::Object { fields } = self.value(entity_id) {
            if !first {
                buffer.put_slice(b",");
            }
            buffer.put_slice(b"{");
            let field_slice = self.object_fields(fields);
            let mut inner_first = true;
            for sf in field_slice {
                if !inner_first {
                    buffer.put_slice(b",");
                }
                inner_first = false;
                buffer.put_slice(keys.serialized_json_key(sf.response_key));
                self.serialize_value_scalar(sf.value, keys, buffer);
            }
            buffer.put_slice(b"}");
            true
        } else {
            false
        }
    }

    /// Hash an entity's selected fields for deduplication.
    pub fn hash_entity_requires(&self, entity_id: FlatValueId, keys: &ResponseKeys) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = ahash::AHasher::default();

        if let FlatValue::Object { fields } = self.value(entity_id) {
            for sf in self.object_fields(fields) {
                keys.key(sf.response_key).hash(&mut hasher);
                self.hash_scalar(sf.value, &mut hasher);
            }
        }
        hasher.finish()
    }

    fn hash_scalar(&self, id: FlatValueId, hasher: &mut impl std::hash::Hasher) {
        use std::hash::Hash;
        match self.value(id) {
            FlatValue::Null => 0u8.hash(hasher),
            FlatValue::Bool(b) => b.hash(hasher),
            FlatValue::I64(n) => n.hash(hasher),
            FlatValue::U64(n) => n.hash(hasher),
            FlatValue::F64(n) => n.to_bits().hash(hasher),
            FlatValue::String(s) => s.hash(hasher),
            FlatValue::RawJson(s) => s.hash(hasher),
            _ => 0u8.hash(hasher),
        }
    }

    fn serialize_value_scalar(&self, id: FlatValueId, _keys: &ResponseKeys, buffer: &mut Vec<u8>) {
        use crate::json_writer::{write_and_escape_string, write_f64, write_i64, write_u64};

        match self.value(id) {
            FlatValue::Null | FlatValue::Missing | FlatValue::Inaccessible => buffer.put(NULL),
            FlatValue::Bool(true) => buffer.put(TRUE),
            FlatValue::Bool(false) => buffer.put(FALSE),
            FlatValue::I64(n) => write_i64(buffer, *n),
            FlatValue::U64(n) => write_u64(buffer, *n),
            FlatValue::F64(n) => write_f64(buffer, *n),
            FlatValue::String(s) => write_and_escape_string(buffer, s),
            FlatValue::RawJson(raw) => buffer.put_slice(raw.as_bytes()),
            _ => buffer.put(NULL),
        }
    }

    pub fn project_requires_to_buffer(
        &self,
        keys: &ResponseKeys,
        possible_types: &PossibleTypes,
        requires_selections: &[SelectionItem],
        entity_id: FlatValueId,
        buffer: &mut Vec<u8>,
        first: bool,
        response_key: Option<&str>,
    ) -> bool {
        match self.value(entity_id) {
            FlatValue::Null | FlatValue::Missing | FlatValue::Inaccessible => false,
            FlatValue::Bool(value) => {
                write_flat_response_key(first, response_key, buffer);
                buffer.put(if *value { TRUE } else { FALSE });
                true
            }
            FlatValue::F64(value) => {
                write_flat_response_key(first, response_key, buffer);
                crate::json_writer::write_f64(buffer, *value);
                true
            }
            FlatValue::I64(value) => {
                write_flat_response_key(first, response_key, buffer);
                crate::json_writer::write_i64(buffer, *value);
                true
            }
            FlatValue::U64(value) => {
                write_flat_response_key(first, response_key, buffer);
                crate::json_writer::write_u64(buffer, *value);
                true
            }
            FlatValue::String(value) => {
                write_flat_response_key(first, response_key, buffer);
                crate::json_writer::write_and_escape_string(buffer, value);
                true
            }
            FlatValue::RawJson(raw) => {
                write_flat_response_key(first, response_key, buffer);
                buffer.put_slice(raw.as_bytes());
                true
            }
            FlatValue::List { items } => {
                write_flat_response_key(first, response_key, buffer);
                buffer.put(OPEN_BRACKET);
                let mut first = true;
                for &item_id in self.list_items(items) {
                    let projected = self.project_requires_to_buffer(
                        keys,
                        possible_types,
                        requires_selections,
                        item_id,
                        buffer,
                        first,
                        None,
                    );
                    if projected {
                        first = false;
                    }
                }
                buffer.put(CLOSE_BRACKET);
                true
            }
            FlatValue::Object { fields } => {
                if requires_selections.is_empty() {
                    write_flat_response_key(first, response_key, buffer);
                    self.serialize_value(entity_id, keys, buffer);
                    return true;
                }

                if fields.is_empty() {
                    return false;
                }

                let parent_first = first;
                let mut first = true;
                self.project_requires_object_to_buffer(
                    keys,
                    possible_types,
                    requires_selections,
                    fields,
                    buffer,
                    &mut first,
                    response_key,
                    parent_first,
                );
                if first {
                    false
                } else {
                    buffer.put(CLOSE_BRACE);
                    true
                }
            }
        }
    }

    fn project_requires_object_to_buffer(
        &self,
        keys: &ResponseKeys,
        possible_types: &PossibleTypes,
        requires_selections: &[SelectionItem],
        fields: &Range<u32>,
        buffer: &mut Vec<u8>,
        first: &mut bool,
        parent_response_key: Option<&str>,
        parent_first: bool,
    ) {
        let type_name = self
            .flat_object_get(fields, keys, TYPENAME_FIELD_NAME)
            .and_then(|id| {
                if let FlatValue::String(value) = self.value(id) {
                    Some(value.as_ref())
                } else {
                    None
                }
            });

        let only_typename = requires_selections.len() == 1
            && requires_selections.iter().all(|selection| {
                matches!(selection, SelectionItem::Field(field) if field.selection_identifier() == TYPENAME_FIELD_NAME)
            });

        if only_typename {
            if let Some(type_name) = type_name {
                write_flat_response_key(parent_first, parent_response_key, buffer);
                buffer.put(OPEN_BRACE);
                write_flat_typename_field(buffer, type_name);
                *first = false;
                return;
            }
        }

        for requires_selection in requires_selections {
            match requires_selection {
                SelectionItem::Field(requires_selection) => {
                    let field_name = requires_selection.name.as_str();
                    let response_key = requires_selection.selection_identifier();

                    if response_key == TYPENAME_FIELD_NAME {
                        continue;
                    }

                    let original = self
                        .flat_object_get(fields, keys, field_name)
                        .or_else(|| self.flat_object_get(fields, keys, response_key));

                    let Some(original) = original else {
                        continue;
                    };

                    let mut object_start_offset = None;

                    if *first {
                        object_start_offset = Some(buffer.len());
                        write_flat_response_key(parent_first, parent_response_key, buffer);
                        buffer.put(OPEN_BRACE);
                        if let Some(type_name) = type_name {
                            write_flat_typename_field(buffer, type_name);
                            *first = false;
                        }
                    }

                    if matches!(
                        self.value(original),
                        FlatValue::Null | FlatValue::Missing | FlatValue::Inaccessible
                    ) {
                        write_flat_response_key(*first, Some(response_key), buffer);
                        buffer.put(NULL);
                        *first = false;
                        continue;
                    }

                    let projected = self.project_requires_to_buffer(
                        keys,
                        possible_types,
                        &requires_selection.selections.items,
                        original,
                        buffer,
                        *first,
                        Some(response_key),
                    );

                    if projected {
                        *first = false;
                    } else if *first {
                        if let Some(offset) = object_start_offset {
                            buffer.truncate(offset);
                        }
                    }
                }
                SelectionItem::InlineFragment(requires_selection) => {
                    let type_condition = &requires_selection.type_condition;
                    let type_name = type_name.unwrap_or(type_condition);
                    if possible_types.entity_satisfies_type_condition(type_name, type_condition)
                        || possible_types.entity_satisfies_type_condition(type_condition, type_name)
                    {
                        self.project_requires_object_to_buffer(
                            keys,
                            possible_types,
                            &requires_selection.selections.items,
                            fields,
                            buffer,
                            first,
                            parent_response_key,
                            parent_first,
                        );
                    }
                }
                SelectionItem::FragmentSpread(_) => {}
            }
        }
    }

    pub fn hash_with_requires(
        &self,
        keys: &ResponseKeys,
        possible_types: &PossibleTypes,
        value_id: FlatValueId,
        requires_selections: &[SelectionItem],
    ) -> u64 {
        let mut hasher = Xxh3::new();
        self.hash_value_with_requires(
            keys,
            possible_types,
            value_id,
            requires_selections,
            &mut hasher,
        );
        hasher.finish()
    }

    fn hash_value_with_requires<H: Hasher>(
        &self,
        keys: &ResponseKeys,
        possible_types: &PossibleTypes,
        value_id: FlatValueId,
        requires_selections: &[SelectionItem],
        state: &mut H,
    ) {
        if requires_selections.is_empty() {
            self.hash_flat_value(keys, value_id, state);
            return;
        }

        match self.value(value_id) {
            FlatValue::Object { fields } => {
                self.hash_object_with_requires(
                    keys,
                    possible_types,
                    fields,
                    requires_selections,
                    state,
                );
            }
            FlatValue::List { items } => {
                for &item_id in self.list_items(items) {
                    self.hash_value_with_requires(
                        keys,
                        possible_types,
                        item_id,
                        requires_selections,
                        state,
                    );
                }
            }
            _ => self.hash_flat_value(keys, value_id, state),
        }
    }

    fn hash_object_with_requires<H: Hasher>(
        &self,
        keys: &ResponseKeys,
        possible_types: &PossibleTypes,
        fields: &Range<u32>,
        requires_selections: &[SelectionItem],
        state: &mut H,
    ) {
        for item in requires_selections {
            match item {
                SelectionItem::Field(field_selection) => {
                    let field_name = field_selection.name.as_str();
                    if let Some((key, value_id)) =
                        self.flat_object_get_entry(fields, keys, field_name)
                    {
                        key.hash(state);
                        self.hash_value_with_requires(
                            keys,
                            possible_types,
                            value_id,
                            &field_selection.selections.items,
                            state,
                        );
                    }
                }
                SelectionItem::InlineFragment(inline_fragment) => {
                    let type_condition = &inline_fragment.type_condition;
                    let type_name = self
                        .flat_object_get(fields, keys, TYPENAME_FIELD_NAME)
                        .and_then(|id| match self.value(id) {
                            FlatValue::String(value) => Some(value.as_ref()),
                            _ => None,
                        })
                        .unwrap_or(type_condition);

                    if possible_types.entity_satisfies_type_condition(type_name, type_condition) {
                        self.hash_object_with_requires(
                            keys,
                            possible_types,
                            fields,
                            &inline_fragment.selections.items,
                            state,
                        );
                    }
                }
                SelectionItem::FragmentSpread(_) => {}
            }
        }
    }

    fn hash_flat_value<H: Hasher>(
        &self,
        keys: &ResponseKeys,
        value_id: FlatValueId,
        state: &mut H,
    ) {
        match self.value(value_id) {
            FlatValue::Null => 0u8.hash(state),
            FlatValue::Bool(value) => value.hash(state),
            FlatValue::I64(value) => value.hash(state),
            FlatValue::U64(value) => value.hash(state),
            FlatValue::F64(value) => value.to_bits().hash(state),
            FlatValue::String(value) => value.hash(state),
            FlatValue::RawJson(value) => value.hash(state),
            FlatValue::Object { fields } => {
                for field in self.object_fields(fields) {
                    keys.key(field.response_key).hash(state);
                    self.hash_flat_value(keys, field.value, state);
                }
            }
            FlatValue::List { items } => {
                for &item_id in self.list_items(items) {
                    self.hash_flat_value(keys, item_id, state);
                }
            }
            FlatValue::Missing => 1u8.hash(state),
            FlatValue::Inaccessible => 2u8.hash(state),
        }
    }

    fn flat_object_get(
        &self,
        fields: &Range<u32>,
        keys: &ResponseKeys,
        key: &str,
    ) -> Option<FlatValueId> {
        self.flat_object_get_entry(fields, keys, key)
            .map(|(_, value)| value)
    }

    fn flat_object_get_entry<'keys>(
        &self,
        fields: &Range<u32>,
        keys: &'keys ResponseKeys,
        key: &str,
    ) -> Option<(&'keys str, FlatValueId)> {
        let key_id = keys.get_key_id(key)?;
        self.object_fields(fields)
            .iter()
            .find(|field| field.response_key == key_id)
            .map(|field| (keys.key(field.response_key), field.value))
    }

    // -- merge helpers (original code continues) --

    /// Return the list of entity ids from the `_entities` field of the data root.
    /// Returns None if there is no `_entities` field or it is not a list.
    pub fn take_entities(
        &self,
        data_root: FlatValueId,
        keys: &ResponseKeys,
    ) -> Option<Vec<FlatValueId>> {
        self.take_entities_by_key(data_root, keys, "_entities")
    }

    pub fn take_entities_by_key(
        &self,
        data_root: FlatValueId,
        keys: &ResponseKeys,
        key: &str,
    ) -> Option<Vec<FlatValueId>> {
        let entities_key = keys.get_key_id(key)?;
        if let FlatValue::Object { fields } = self.value(data_root) {
            for sf in self.object_fields(fields) {
                if sf.response_key == entities_key {
                    if let FlatValue::List { items } = self.value(sf.value) {
                        return Some(self.list_items(items).to_vec());
                    }
                }
            }
        }
        None
    }

    // -- merge helpers --

    /// Absorb all retained bytes from another store so borrowed strings/raw-json stay valid.
    pub fn absorb_bytes_from(&mut self, other: &mut FlatResponseStore<'static>) {
        self.retained_bytes.append(&mut other.retained_bytes);
    }

    /// Move all values from another store into this store and remap store-local ids by offset.
    ///
    /// FlatValueId, object field ranges, and list item ranges are store-local. Appending a whole
    /// store lets execution merge subgraph responses without recursively cloning each subtree.
    pub fn append_store(&mut self, mut source: FlatResponseStore<'a>) -> FlatStoreAppendMap {
        let value_offset = self.values.len() as u32;
        let object_field_offset = self.object_fields.len() as u32;
        let list_item_offset = self.list_items.len() as u32;

        for value in &mut source.values {
            match value {
                FlatValue::Object { fields } => {
                    fields.start += object_field_offset;
                    fields.end += object_field_offset;
                }
                FlatValue::List { items } => {
                    items.start += list_item_offset;
                    items.end += list_item_offset;
                }
                _ => {}
            }
        }

        for field in &mut source.object_fields {
            field.value.0 += value_offset;
        }

        for item in &mut source.list_items {
            item.0 += value_offset;
        }

        self.values.append(&mut source.values);
        self.object_fields.append(&mut source.object_fields);
        self.list_items.append(&mut source.list_items);
        self.retained_bytes.append(&mut source.retained_bytes);

        FlatStoreAppendMap { value_offset }
    }

    /// Recursively import a value tree from another store into this one,
    /// producing a new FlatValueId. All borrowed values stay valid if the
    /// source retained bytes are absorbed into this store first.
    pub fn import_value_tree(
        &mut self,
        source: &FlatResponseStore<'static>,
        source_id: FlatValueId,
    ) -> FlatValueId {
        self.import_value_tree_impl(source, source_id)
    }

    fn import_value_tree_impl(
        &mut self,
        source: &FlatResponseStore<'static>,
        source_id: FlatValueId,
    ) -> FlatValueId {
        match source.value(source_id) {
            FlatValue::Null => self.alloc_null(),
            FlatValue::Bool(b) => self.alloc_bool(*b),
            FlatValue::I64(n) => self.alloc_i64(*n),
            FlatValue::U64(n) => self.alloc_u64(*n),
            FlatValue::F64(n) => self.alloc_f64(*n),
            FlatValue::String(s) => self.alloc_string(s.clone()),
            FlatValue::RawJson(raw) => self.alloc_raw_json(raw.clone()),
            FlatValue::Missing => self.alloc_missing(),
            FlatValue::Inaccessible => self.alloc_inaccessible(),
            FlatValue::Object { fields: range } => {
                let src_fields = source.object_fields(range);
                let mut dst_fields = Vec::with_capacity(src_fields.len());
                for sf in src_fields {
                    let imported_value = self.import_value_tree_impl(source, sf.value);
                    dst_fields.push(FlatObjectField {
                        response_key: sf.response_key,
                        value: imported_value,
                        output_key: sf.output_key,
                        output_position: sf.output_position,
                        is_non_null: sf.is_non_null,
                    });
                }
                self.alloc_object(dst_fields)
            }
            FlatValue::List { items: range } => {
                let src_items = source.list_items(range);
                let mut dst_items = Vec::with_capacity(src_items.len());
                for &si in src_items {
                    dst_items.push(self.import_value_tree_impl(source, si));
                }
                self.alloc_list_from_items(dst_items)
            }
        }
    }

    pub fn merge_value(&mut self, target_id: FlatValueId, source_id: FlatValueId) {
        let source_val = self.value(source_id).clone();
        let target_clone = self.value(target_id).clone();

        match (target_clone, source_val) {
            (
                FlatValue::Object {
                    fields: target_range,
                },
                FlatValue::Object {
                    fields: source_range,
                },
            ) => {
                let source_fields = self.object_fields
                    [source_range.start as usize..source_range.end as usize]
                    .to_vec();
                let target_fields =
                    &self.object_fields[target_range.start as usize..target_range.end as usize];
                let mut new_fields = target_fields.to_vec();

                if new_fields.len() >= 8 && source_fields.len() >= 2 {
                    let mut target_index_by_key = HashMap::with_capacity(new_fields.len());
                    for (index, field) in new_fields.iter().enumerate() {
                        target_index_by_key
                            .entry(field.response_key)
                            .or_insert(index);
                    }

                    for sf in source_fields {
                        if let Some(&index) = target_index_by_key.get(&sf.response_key) {
                            let existing_value = new_fields[index].value;
                            self.merge_value(existing_value, sf.value);
                        } else {
                            target_index_by_key.insert(sf.response_key, new_fields.len());
                            new_fields.push(sf);
                        }
                    }
                } else {
                    for sf in source_fields {
                        if let Some(existing) = new_fields
                            .iter_mut()
                            .find(|f| f.response_key == sf.response_key)
                        {
                            self.merge_value(existing.value, sf.value);
                        } else {
                            new_fields.push(sf);
                        }
                    }
                }

                let start = self.object_fields.len() as u32;
                let end = start + new_fields.len() as u32;
                self.object_fields.extend(new_fields);
                self.values[target_id.0 as usize] = FlatValue::Object { fields: start..end };
            }
            (
                FlatValue::List {
                    items: target_range,
                },
                FlatValue::List {
                    items: source_range,
                },
            ) => {
                let source_items = self.list_items
                    [source_range.start as usize..source_range.end as usize]
                    .to_vec();
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
        if let FlatValue::Object {
            fields: target_range,
        } = self.value(object_id).clone()
        {
            let target_fields =
                self.object_fields[target_range.start as usize..target_range.end as usize].to_vec();
            let mut new_fields = target_fields;
            for field in fields {
                if !new_fields
                    .iter()
                    .any(|f| f.response_key == field.response_key)
                {
                    new_fields.push(field);
                }
            }
            let start = self.object_fields.len() as u32;
            let end = start + new_fields.len() as u32;
            self.object_fields.extend(new_fields);
            self.values[object_id.0 as usize] = FlatValue::Object { fields: start..end };
        }
    }

    pub fn sort_object_fields_by_output_position(&mut self, object_id: FlatValueId) {
        if let FlatValue::Object {
            fields: target_range,
        } = self.value(object_id).clone()
        {
            let range_start = target_range.start as usize;
            let range_end = target_range.end as usize;
            let field_slice = &mut self.object_fields[range_start..range_end];
            field_slice.sort_by_key(|f| (f.output_position, f.output_key));
            // After sorting by position, assign sequential output_position values
            // for fields that have one, to normalize after merges.
            let mut pos: u16 = 0;
            for sf in field_slice.iter_mut() {
                if sf.output_key.is_some() {
                    sf.output_position = Some(pos);
                    pos += 1;
                }
            }
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
    items: Vec<FlatValueId>,
    len: u32,
}

fn write_flat_response_key(first: bool, response_key: Option<&str>, buffer: &mut Vec<u8>) {
    if !first {
        buffer.put(COMMA);
    }
    if let Some(response_key) = response_key {
        buffer.put(QUOTE);
        buffer.put(response_key.as_bytes());
        buffer.put(QUOTE);
        buffer.put(COLON);
    }
}

fn write_flat_typename_field(buffer: &mut Vec<u8>, type_name: &str) {
    buffer.put(TYPENAME_JSON_FIELD);
    crate::json_writer::write_and_escape_string(buffer, type_name);
}

// -- Serializer --

use crate::json_writer::{write_and_escape_string, write_f64, write_i64, write_u64};
impl<'a> FlatResponseStore<'a> {
    /// Serialize a value into a JSON buffer using the provided key table.
    pub fn serialize_value(&self, id: FlatValueId, keys: &ResponseKeys, buffer: &mut Vec<u8>) {
        self.serialize_value_impl(id, keys, buffer);
    }

    fn serialize_value_impl(&self, id: FlatValueId, keys: &ResponseKeys, buffer: &mut Vec<u8>) {
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

    fn serialize_object(&self, fields: &Range<u32>, keys: &ResponseKeys, buffer: &mut Vec<u8>) {
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

    fn serialize_list(&self, items: &Range<u32>, keys: &ResponseKeys, buffer: &mut Vec<u8>) {
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

    pub fn serialize_output_value_impl(
        &self,
        id: FlatValueId,
        keys: &ResponseKeys,
        buffer: &mut Vec<u8>,
    ) -> bool {
        match self.value(id) {
            FlatValue::Null | FlatValue::Missing | FlatValue::Inaccessible => {
                buffer.put(NULL);
                false
            }
            FlatValue::Bool(true) => {
                buffer.put(TRUE);
                true
            }
            FlatValue::Bool(false) => {
                buffer.put(FALSE);
                true
            }
            FlatValue::I64(num) => {
                write_i64(buffer, *num);
                true
            }
            FlatValue::U64(num) => {
                write_u64(buffer, *num);
                true
            }
            FlatValue::F64(num) => {
                write_f64(buffer, *num);
                true
            }
            FlatValue::String(s) => {
                write_and_escape_string(buffer, s);
                true
            }
            FlatValue::RawJson(raw) => {
                buffer.put_slice(raw.as_bytes());
                true
            }
            FlatValue::Object { fields } => self.serialize_output_object(fields, keys, buffer),
            FlatValue::List { items } => self.serialize_output_list(items, keys, buffer),
        }
    }

    fn serialize_output_object(
        &self,
        fields: &Range<u32>,
        keys: &ResponseKeys,
        buffer: &mut Vec<u8>,
    ) -> bool {
        let checkpoint = buffer.len();
        buffer.put(OPEN_BRACE);
        let field_slice = self.object_fields(fields);
        let mut first = true;

        for field in field_slice {
            let output_key = match field.output_key {
                Some(k) => k,
                None => continue,
            };
            if !first {
                buffer.put(COMMA);
            }
            first = false;

            buffer.put_slice(keys.serialized_json_key(output_key));
            let ok = self.serialize_output_value_impl(field.value, keys, buffer);
            if !ok && field.is_non_null {
                buffer.truncate(checkpoint);
                buffer.put(NULL);
                return false;
            }
        }

        buffer.put(CLOSE_BRACE);
        true
    }

    fn serialize_output_list(
        &self,
        items: &Range<u32>,
        keys: &ResponseKeys,
        buffer: &mut Vec<u8>,
    ) -> bool {
        buffer.put(OPEN_BRACKET);
        let item_slice = self.list_items(items);
        let mut first = true;
        for &item_id in item_slice {
            if !first {
                buffer.put(COMMA);
            }
            first = false;
            self.serialize_output_value_impl(item_id, keys, buffer);
        }
        buffer.put(CLOSE_BRACKET);
        true
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
        let obj = store.alloc_object(vec![FlatObjectField::new(k_name, v_name)]);

        let FlatValue::Object { fields } = store.value(obj) else {
            panic!("expected object");
        };
        let field_slice = store.object_fields(fields);
        assert_eq!(field_slice.len(), 1);
        assert_eq!(field_slice[0].response_key, k_name);
        assert!(matches!(
            store.value(field_slice[0].value),
            FlatValue::String(s) if s == "Alice"
        ));
    }

    #[test]
    fn traverse_type_condition_matches_when_typename_is_missing() {
        let mut store = FlatResponseStore::new();
        let keys = ResponseKeys::default();
        let obj = store.alloc_object(vec![]);
        let mut type_conditions = std::collections::BTreeSet::new();
        type_conditions.insert("User".to_string());
        let path = vec![FlattenNodePathSegment::TypeCondition(type_conditions)];

        let mut visited = Vec::new();
        store.traverse_path(
            obj,
            &keys,
            &path,
            0,
            &PossibleTypes::default(),
            &mut |id, _, _| visited.push(id),
        );

        assert_eq!(visited, vec![obj]);
    }

    #[test]
    fn traverse_type_condition_skips_when_typename_does_not_match() {
        let mut store = FlatResponseStore::new();
        let mut keys = ResponseKeys::default();
        let typename_key = keys.intern("__typename");
        let typename = store.alloc_string(Cow::Borrowed("Product"));
        let obj = store.alloc_object(vec![FlatObjectField::new(typename_key, typename)]);
        let mut type_conditions = std::collections::BTreeSet::new();
        type_conditions.insert("User".to_string());
        let path = vec![FlattenNodePathSegment::TypeCondition(type_conditions)];

        let mut visited = Vec::new();
        store.traverse_path(
            obj,
            &keys,
            &path,
            0,
            &PossibleTypes::default(),
            &mut |id, _, _| visited.push(id),
        );

        assert!(visited.is_empty());
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
    fn nested_list_builders_do_not_overlap_ranges() {
        let mut store = FlatResponseStore::new();
        let mut outer = store.reserve_list(1);

        let mut inner = store.reserve_list(2);
        let one = store.alloc_i64(1);
        store.push_list_item(&mut inner, one);
        let two = store.alloc_i64(2);
        store.push_list_item(&mut inner, two);
        let inner_list = store.finish_list(inner);

        store.push_list_item(&mut outer, inner_list);
        let outer_list = store.finish_list(outer);

        let FlatValue::List { items } = store.value(outer_list) else {
            panic!("expected outer list");
        };
        assert_eq!(store.list_items(items), &[inner_list]);
    }

    #[test]
    fn append_store_remaps_nested_ids_and_ranges() {
        let mut keys = ResponseKeys::default();
        let k_items = keys.intern("items");
        let k_name = keys.intern("name");

        let mut target = FlatResponseStore::new();
        let existing = target.alloc_string(Cow::Borrowed("existing"));

        let mut source = FlatResponseStore::new();
        let name = source.alloc_string(Cow::Borrowed("Alice"));
        let item = source.alloc_object(vec![FlatObjectField::new(k_name, name)]);
        let list = source.alloc_list_from_items(vec![item]);
        let root = source.alloc_object(vec![FlatObjectField::new(k_items, list)]);

        let append_map = target.append_store(source);
        let moved_root = append_map.value(root);

        assert!(matches!(target.value(existing), FlatValue::String(value) if value == "existing"));
        let FlatValue::Object { fields } = target.value(moved_root) else {
            panic!("expected moved root object");
        };
        let moved_list = target.object_fields(fields)[0].value;
        let FlatValue::List { items } = target.value(moved_list) else {
            panic!("expected moved list");
        };
        let moved_item = target.list_items(items)[0];
        let FlatValue::Object { fields } = target.value(moved_item) else {
            panic!("expected moved item object");
        };
        let moved_name = target.object_fields(fields)[0].value;
        assert!(matches!(target.value(moved_name), FlatValue::String(value) if value == "Alice"));
    }

    #[test]
    fn merge_objects() {
        let mut store = FlatResponseStore::new();
        let k1 = ResponseKeyId(0);
        let k2 = ResponseKeyId(1);

        let v1 = store.alloc_string(Cow::Borrowed("a"));
        let obj1 = store.alloc_object(vec![FlatObjectField::new(k1, v1)]);

        let v2 = store.alloc_string(Cow::Borrowed("b"));
        let obj2 = store.alloc_object(vec![FlatObjectField::new(k2, v2)]);

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
            FlatObjectField::new(k_name, v_name),
            FlatObjectField::new(k_age, v_age),
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
        let obj = store.alloc_object(vec![FlatObjectField::new(k_x, v_x)]);

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
