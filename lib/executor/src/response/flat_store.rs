use std::borrow::Cow;
use std::hash::{Hash, Hasher};

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

// ---------------------------------------------------------------------------
// Id types
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Value types
// ---------------------------------------------------------------------------

/// A leaf value in the flat store.
///
/// Objects own their fields via `Box<[FlatObjectField]>` and lists own their
/// items via `Box<[FlatValueId]>`.  No paired global vectors — each compound
/// value is self-contained, eliminating the shared grow/realloc paths.
#[derive(Debug, Clone)]
pub enum FlatValue<'a> {
    Null,
    Bool(bool),
    I64(i64),
    U64(u64),
    F64(f64),
    String(Cow<'a, str>),
    RawJson(Cow<'a, str>),
    Object { fields: Box<[FlatObjectField]> },
    List { items: Box<[FlatValueId]> },
    Missing,
    Inaccessible,
}

/// One field of a flat object.
///
/// Fields within an object are always kept **sorted by `response_key`** so that
/// lookups can use binary search and merges can use a two-pointer walk.
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

// ---------------------------------------------------------------------------
// Append map (remap value ids when concatenating stores)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
pub struct FlatStoreAppendMap {
    value_offset: u32,
}

impl FlatStoreAppendMap {
    pub fn value(&self, id: FlatValueId) -> FlatValueId {
        FlatValueId(id.0 + self.value_offset)
    }
}

// ---------------------------------------------------------------------------
// Response key table
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Flat response store
// ---------------------------------------------------------------------------

/// The normalized flat response store.
///
/// All response data is stored as a linear array of values.  Objects and lists
/// own their content directly via boxed slices — there are no shared backing
/// vectors for fields or items.
#[derive(Debug, Clone, Default)]
pub struct FlatResponseStore<'a> {
    values: Vec<FlatValue<'a>>,
    /// Bytes that owned strings/raw-json borrow from.
    retained_bytes: Vec<bytes::Bytes>,
}

impl<'a> FlatResponseStore<'a> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_capacity(values: usize, retained_bytes: usize) -> Self {
        Self {
            values: Vec::with_capacity(values),
            retained_bytes: Vec::with_capacity(retained_bytes),
        }
    }

    pub fn with_response_size_hint(response_size: usize) -> Self {
        let values = (response_size / 20).clamp(32, 8192);
        Self::with_capacity(values, 1)
    }

    pub fn reserve_values(&mut self, additional: usize) {
        self.values.reserve(additional);
    }

    // -- allocation helpers --

    fn push_value(&mut self, value: FlatValue<'a>) -> FlatValueId {
        let id = FlatValueId(self.values.len() as u32);
        self.values.push(value);
        id
    }

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

    /// Push an object with sorted fields.  The caller must ensure fields are
    /// already sorted by `response_key` (or pass an unsorted Vec — this method
    /// will sort it).
    pub fn alloc_object(&mut self, mut fields: Vec<FlatObjectField>) -> FlatValueId {
        // Sort after possibly unsorted vecs.
        fields.sort_unstable_by_key(|f| f.response_key);
        self.push_value(FlatValue::Object {
            fields: fields.into_boxed_slice(),
        })
    }

    /// Push a list with the given items.
    pub fn alloc_list_from_items(&mut self, items: Vec<FlatValueId>) -> FlatValueId {
        self.push_value(FlatValue::List {
            items: items.into_boxed_slice(),
        })
    }

    /// Allocate a list directly from a boxed slice (for zero-copy reuse).
    pub fn alloc_list_from_boxed(&mut self, items: Box<[FlatValueId]>) -> FlatValueId {
        self.push_value(FlatValue::List { items })
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

    pub fn set_value(&mut self, id: FlatValueId, value: FlatValue<'a>) {
        self.values[id.0 as usize] = value;
    }

    /// Binary search for a field by response key.  Fields must be sorted.
    #[inline]
    pub fn find_field_by_key(
        fields: &[FlatObjectField],
        key_id: ResponseKeyId,
    ) -> Option<&FlatObjectField> {
        fields
            .binary_search_by_key(&key_id, |f| f.response_key)
            .ok()
            .map(|idx| &fields[idx])
    }

    /// Linear fallback search (accepts unsorted).
    pub fn find_field_linear(
        fields: &[FlatObjectField],
        key_id: ResponseKeyId,
    ) -> Option<&FlatObjectField> {
        fields.iter().find(|f| f.response_key == key_id)
    }

    // -- mutation helpers --

    pub fn set_object_field_value(
        &mut self,
        object_id: FlatValueId,
        key_id: ResponseKeyId,
        new_value: FlatValueId,
    ) {
        if let FlatValue::Object { fields } = self.value(object_id).clone() {
            let mut fields_vec = fields.into_vec();
            if let Some(sf) = fields_vec.iter_mut().find(|f| f.response_key == key_id) {
                sf.value = new_value;
            }
            self.values[object_id.0 as usize] = FlatValue::Object {
                fields: fields_vec.into_boxed_slice(),
            };
        }
    }

    pub fn rename_object_field(
        &mut self,
        object_id: FlatValueId,
        old_key_id: ResponseKeyId,
        new_key_id: ResponseKeyId,
    ) {
        if let FlatValue::Object { fields } = self.value(object_id).clone() {
            let mut fields_vec = fields.into_vec();
            if let Some(sf) = fields_vec.iter_mut().find(|f| f.response_key == old_key_id) {
                sf.response_key = new_key_id;
            }
            fields_vec.sort_unstable_by_key(|f| f.response_key);
            self.values[object_id.0 as usize] = FlatValue::Object {
                fields: fields_vec.into_boxed_slice(),
            };
        }
    }

    // -- path resolution --

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
            FlatValue::Object { fields } => {
                let mut results = Vec::new();
                for sf in fields.iter() {
                    if sf.response_key == key_id {
                        results.extend(self.resolve_path_impl(sf.value, keys, segments, depth + 1));
                    }
                }
                results
            }
            FlatValue::List { items } => {
                let mut results = Vec::new();
                for &item_id in items.iter() {
                    results.extend(self.resolve_path_impl(item_id, keys, segments, depth));
                }
                results
            }
            _ => Vec::new(),
        }
    }

    // -- path traversal --

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
                    for &item_id in items.iter() {
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
                    for &item_id in items.iter() {
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
                    for sf in fields.iter() {
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
                    let type_name = fields.iter().find_map(|sf| {
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
                    for &item_id in items.iter() {
                        self.traverse_path(item_id, keys, path, depth, possible_types, callback);
                    }
                }
                _ => {}
            },
        }
    }

    /// Get the `__typename` of a flat object, if present.
    pub fn get_typename(&self, object_id: FlatValueId, keys: &ResponseKeys) -> Option<&str> {
        if let FlatValue::Object { fields } = self.value(object_id) {
            let tk_id = keys.get_key_id("__typename")?;
            for sf in fields.iter() {
                if sf.response_key == tk_id {
                    if let FlatValue::String(s) = self.value(sf.value) {
                        return Some(s.as_ref());
                    }
                }
            }
        }
        None
    }

    // -- entity-requires serialization / hashing --

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
            let mut inner_first = true;
            for sf in fields.iter() {
                if !inner_first {
                    buffer.put_slice(b",");
                }
                inner_first = false;
                buffer.put_slice(keys.serialized_json_key(sf.response_key));
                self.serialize_value_scalar(sf.value, buffer);
            }
            buffer.put_slice(b"}");
            true
        } else {
            false
        }
    }

    pub fn hash_entity_requires(&self, entity_id: FlatValueId, keys: &ResponseKeys) -> u64 {
        let mut hasher = ahash::AHasher::default();

        if let FlatValue::Object { fields } = self.value(entity_id) {
            for sf in fields.iter() {
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

    fn serialize_value_scalar(&self, id: FlatValueId, buffer: &mut Vec<u8>) {
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
                write_f64(buffer, *value);
                true
            }
            FlatValue::I64(value) => {
                write_flat_response_key(first, response_key, buffer);
                write_i64(buffer, *value);
                true
            }
            FlatValue::U64(value) => {
                write_flat_response_key(first, response_key, buffer);
                write_u64(buffer, *value);
                true
            }
            FlatValue::String(value) => {
                write_flat_response_key(first, response_key, buffer);
                write_and_escape_string(buffer, value);
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
                let mut list_first = true;
                for &item_id in items.iter() {
                    let projected = self.project_requires_to_buffer(
                        keys,
                        possible_types,
                        requires_selections,
                        item_id,
                        buffer,
                        list_first,
                        None,
                    );
                    if projected {
                        list_first = false;
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
                let mut inner_first = true;
                self.project_requires_object_to_buffer(
                    keys,
                    possible_types,
                    requires_selections,
                    fields,
                    buffer,
                    &mut inner_first,
                    response_key,
                    parent_first,
                );
                if inner_first {
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
        fields: &[FlatObjectField],
        buffer: &mut Vec<u8>,
        first: &mut bool,
        parent_response_key: Option<&str>,
        parent_first: bool,
    ) {
        let type_name = Self::find_field_by_key(
            fields,
            keys.get_key_id(TYPENAME_FIELD_NAME)
                .unwrap_or(ResponseKeyId(0)),
        )
        .and_then(|sf| {
            if let FlatValue::String(value) = self.value(sf.value) {
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

                    let original = Self::find_field_by_key(
                        fields,
                        keys.get_key_id(field_name)
                            .or_else(|| keys.get_key_id(response_key))
                            .unwrap_or(ResponseKeyId(u32::MAX)),
                    );

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
                        self.value(original.value),
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
                        original.value,
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

    // -- hashing with requires --

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
                for &item_id in items.iter() {
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
        fields: &[FlatObjectField],
        requires_selections: &[SelectionItem],
        state: &mut H,
    ) {
        for item in requires_selections {
            match item {
                SelectionItem::Field(field_selection) => {
                    let field_name = field_selection.name.as_str();
                    if let Some(key_id) = keys.get_key_id(field_name) {
                        if let Some(sf) = Self::find_field_by_key(fields, key_id) {
                            keys.key(sf.response_key).hash(state);
                            self.hash_value_with_requires(
                                keys,
                                possible_types,
                                sf.value,
                                &field_selection.selections.items,
                                state,
                            );
                        }
                    }
                }
                SelectionItem::InlineFragment(inline_fragment) => {
                    let type_condition = &inline_fragment.type_condition;
                    let type_name = {
                        let tk = keys.get_key_id(TYPENAME_FIELD_NAME);
                        tk.and_then(|k| Self::find_field_by_key(fields, k))
                            .and_then(|sf| match self.value(sf.value) {
                                FlatValue::String(value) => Some(value.as_ref()),
                                _ => None,
                            })
                            .unwrap_or(type_condition)
                    };

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
                for field in fields.iter() {
                    keys.key(field.response_key).hash(state);
                    self.hash_flat_value(keys, field.value, state);
                }
            }
            FlatValue::List { items } => {
                for &item_id in items.iter() {
                    self.hash_flat_value(keys, item_id, state);
                }
            }
            FlatValue::Missing => 1u8.hash(state),
            FlatValue::Inaccessible => 2u8.hash(state),
        }
    }

    // -- entities extraction --

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
            for sf in fields.iter() {
                if sf.response_key == entities_key {
                    if let FlatValue::List { items } = self.value(sf.value) {
                        return Some(items.to_vec());
                    }
                }
            }
        }
        None
    }

    // -- merge --

    pub fn absorb_bytes_from(&mut self, other: &mut FlatResponseStore<'static>) {
        self.retained_bytes.append(&mut other.retained_bytes);
    }

    /// Move all values from another store into this one, remapping ids.
    /// Object field values and list items are index-offset into the target
    /// `values` Vec.
    pub fn append_store(&mut self, mut source: FlatResponseStore<'a>) -> FlatStoreAppendMap {
        let value_offset = self.values.len() as u32;

        for value in &mut source.values {
            match value {
                FlatValue::Object { fields } => {
                    for sf in fields.iter_mut() {
                        sf.value.0 += value_offset;
                    }
                }
                FlatValue::List { items } => {
                    for item in items.iter_mut() {
                        item.0 += value_offset;
                    }
                }
                _ => {}
            }
        }

        self.values.append(&mut source.values);
        self.retained_bytes.append(&mut source.retained_bytes);

        FlatStoreAppendMap { value_offset }
    }

    /// Recursively import a value tree from another store into this one.
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
            FlatValue::Object { fields } => {
                let mut dst_fields = Vec::with_capacity(fields.len());
                for sf in fields.iter() {
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
            FlatValue::List { items } => {
                let mut dst_items = Vec::with_capacity(items.len());
                for &si in items.iter() {
                    dst_items.push(self.import_value_tree_impl(source, si));
                }
                self.alloc_list_from_items(dst_items)
            }
        }
    }

    /// Two-pointer merge of sorted object fields (target + source).
    /// Both field slices must be sorted by `response_key`.
    /// Modifies the target value in place.
    pub fn merge_value(&mut self, target_id: FlatValueId, source_id: FlatValueId) {
        let source_val = self.value(source_id).clone();
        let target_clone = self.value(target_id).clone();

        match (target_clone, source_val) {
            (
                FlatValue::Object {
                    fields: target_fields,
                },
                FlatValue::Object {
                    fields: source_fields,
                },
            ) => {
                // Recursively merge values at matching keys.
                let mut ti = 0usize;
                let mut si = 0usize;
                while ti < target_fields.len() && si < source_fields.len() {
                    match target_fields[ti]
                        .response_key
                        .cmp(&source_fields[si].response_key)
                    {
                        std::cmp::Ordering::Less => ti += 1,
                        std::cmp::Ordering::Greater => si += 1,
                        std::cmp::Ordering::Equal => {
                            self.merge_value(target_fields[ti].value, source_fields[si].value);
                            ti += 1;
                            si += 1;
                        }
                    }
                }

                // Rebuild: two-pointer merge of sorted slices, keeping target's
                // (now-updated) value for colliding keys.
                let mut result = Vec::with_capacity(target_fields.len() + source_fields.len());
                let mut ti = 0usize;
                let mut si = 0usize;
                while ti < target_fields.len() || si < source_fields.len() {
                    if si >= source_fields.len() {
                        result.push(target_fields[ti].clone());
                        ti += 1;
                    } else if ti >= target_fields.len() {
                        result.push(source_fields[si].clone());
                        si += 1;
                    } else {
                        match target_fields[ti]
                            .response_key
                            .cmp(&source_fields[si].response_key)
                        {
                            std::cmp::Ordering::Less => {
                                result.push(target_fields[ti].clone());
                                ti += 1;
                            }
                            std::cmp::Ordering::Greater => {
                                result.push(source_fields[si].clone());
                                si += 1;
                            }
                            std::cmp::Ordering::Equal => {
                                result.push(FlatObjectField {
                                    value: target_fields[ti].value,
                                    ..target_fields[ti].clone()
                                });
                                ti += 1;
                                si += 1;
                            }
                        }
                    }
                }

                self.values[target_id.0 as usize] = FlatValue::Object {
                    fields: result.into_boxed_slice(),
                };
            }
            (
                FlatValue::List {
                    items: target_items,
                },
                FlatValue::List {
                    items: source_items,
                },
            ) => {
                let mut result = target_items.to_vec();
                for (ti, si) in result.iter_mut().zip(source_items.iter()) {
                    self.merge_value(*ti, *si);
                }
                self.values[target_id.0 as usize] = FlatValue::List {
                    items: result.into_boxed_slice(),
                };
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
        new_fields: impl IntoIterator<Item = FlatObjectField>,
    ) {
        if let FlatValue::Object { fields } = self.value(object_id).clone() {
            let mut vec = fields.into_vec();
            for field in new_fields {
                if !vec.iter().any(|f| f.response_key == field.response_key) {
                    vec.push(field);
                }
            }
            vec.sort_unstable_by_key(|f| f.response_key);
            self.values[object_id.0 as usize] = FlatValue::Object {
                fields: vec.into_boxed_slice(),
            };
        }
    }

    pub fn sort_object_fields_by_output_position(&mut self, object_id: FlatValueId) {
        if let FlatValue::Object { fields } = self.value(object_id).clone() {
            let mut field_vec = fields.into_vec();
            field_vec.sort_by_key(|f| (f.output_position, f.output_key));
            let mut pos: u16 = 0;
            for sf in field_vec.iter_mut() {
                if sf.output_key.is_some() {
                    sf.output_position = Some(pos);
                    pos += 1;
                }
            }
            self.values[object_id.0 as usize] = FlatValue::Object {
                fields: field_vec.into_boxed_slice(),
            };
        }
    }

    pub fn clear(&mut self) {
        self.values.clear();
        self.retained_bytes.clear();
    }

    // -- serialization --

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

    fn serialize_object(
        &self,
        fields: &[FlatObjectField],
        keys: &ResponseKeys,
        buffer: &mut Vec<u8>,
    ) {
        buffer.put(OPEN_BRACE);
        let mut first = true;
        for field in fields.iter() {
            if !first {
                buffer.put(COMMA);
            }
            first = false;
            buffer.put_slice(keys.serialized_json_key(field.response_key));
            self.serialize_value_impl(field.value, keys, buffer);
        }
        buffer.put(CLOSE_BRACE);
    }

    fn serialize_list(&self, items: &[FlatValueId], keys: &ResponseKeys, buffer: &mut Vec<u8>) {
        buffer.put(OPEN_BRACKET);
        let mut first = true;
        for &item_id in items.iter() {
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
        fields: &[FlatObjectField],
        keys: &ResponseKeys,
        buffer: &mut Vec<u8>,
    ) -> bool {
        let checkpoint = buffer.len();
        buffer.put(OPEN_BRACE);
        let mut first = true;

        for field in fields.iter() {
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
        items: &[FlatValueId],
        keys: &ResponseKeys,
        buffer: &mut Vec<u8>,
    ) -> bool {
        buffer.put(OPEN_BRACKET);
        let mut first = true;
        for &item_id in items.iter() {
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

// -- helpers --

/// Merge two sorted field slices via two-pointer walk.
/// Both slices MUST be sorted by `response_key`.
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
    write_and_escape_string(buffer, type_name);
}

use crate::json_writer::{write_and_escape_string, write_f64, write_i64, write_u64};

// -- top-level serialization --

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
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].response_key, k_name);
        assert!(matches!(
            store.value(fields[0].value),
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
        assert_eq!(items.len(), 2);
        assert!(matches!(store.value(items[0]), FlatValue::I64(1)));
        assert!(matches!(store.value(items[1]), FlatValue::I64(2)));
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
        let moved_list = fields[0].value;
        let FlatValue::List { items } = target.value(moved_list) else {
            panic!("expected moved list");
        };
        let moved_item = items[0];
        let FlatValue::Object { fields } = target.value(moved_item) else {
            panic!("expected moved item object");
        };
        let moved_name = fields[0].value;
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
        assert_eq!(fields.len(), 2);
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
