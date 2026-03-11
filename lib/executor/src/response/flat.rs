use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt;
use std::hash::{Hash, Hasher};

use ahash::AHashMap;
use hive_router_query_planner::planner::plan_nodes::{
    FetchNodePathSegment, FetchRewrite, FieldSymbolId, FlattenNodePathSegment, KeyRenamer,
    RuntimeCompiledSelectionItem, RuntimeCompiledSelectionSet, SchemaInterner, TypeId, ValueSetter,
};
use serde::de::{self, DeserializeSeed, Deserializer, MapAccess, SeqAccess, Visitor};
use sonic_rs::{JsonNumberTrait, ValueRef};
use xxhash_rust::xxh3::Xxh3;

use crate::{
    introspection::schema::PossibleTypes,
    response::graphql_error::{GraphQLErrorPath, GraphQLErrorPathSegment},
    utils::consts::TYPENAME_FIELD_NAME,
};

#[derive(Clone, Copy, Debug)]
pub struct ValueId(pub usize);

#[derive(Clone, Copy, Debug)]
pub struct ObjectId(pub usize);

#[derive(Clone, Copy, Debug)]
pub struct ListId(pub usize);

#[derive(Clone, Copy, Debug)]
pub struct PathNodeId(pub u32);

#[derive(Clone, Debug)]
enum PathSeg {
    Field(FieldSymbolId),
    Index(usize),
}

#[derive(Clone, Debug)]
struct PathNode {
    parent: Option<PathNodeId>,
    seg: PathSeg,
}

#[derive(Clone, Debug, Default)]
pub struct PathArena {
    nodes: Vec<PathNode>,
    field_symbols: HashMap<String, FieldSymbolId>,
    field_names: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct FlattenEntityTargets {
    pub target_value_ids: Vec<ValueId>,
    pub path_ids: Vec<PathNodeId>,
    pub path_arena: PathArena,
}

impl PathArena {
    fn intern_field(&mut self, field: &str) -> FieldSymbolId {
        if let Some(id) = self.field_symbols.get(field) {
            return *id;
        }
        let id = FieldSymbolId(self.field_names.len() as u32);
        self.field_names.push(field.to_string());
        self.field_symbols.insert(field.to_string(), id);
        id
    }

    fn push_index(&mut self, parent: Option<PathNodeId>, index: usize) -> PathNodeId {
        let id = PathNodeId(self.nodes.len() as u32);
        self.nodes.push(PathNode {
            parent,
            seg: PathSeg::Index(index),
        });
        id
    }

    fn push_field(&mut self, parent: Option<PathNodeId>, field: &str) -> PathNodeId {
        let symbol = self.intern_field(field);
        let id = PathNodeId(self.nodes.len() as u32);
        self.nodes.push(PathNode {
            parent,
            seg: PathSeg::Field(symbol),
        });
        id
    }

    pub fn materialize(&self, node_id: PathNodeId) -> GraphQLErrorPath {
        let mut segments = Vec::new();
        let mut current = Some(node_id);
        while let Some(id) = current {
            let Some(node) = self.nodes.get(id.0 as usize) else {
                break;
            };
            match node.seg {
                PathSeg::Field(field_id) => {
                    let field = self
                        .field_names
                        .get(field_id.0 as usize)
                        .cloned()
                        .unwrap_or_default();
                    segments.push(GraphQLErrorPathSegment::String(field));
                }
                PathSeg::Index(index) => segments.push(GraphQLErrorPathSegment::Index(index)),
            }
            current = node.parent;
        }
        segments.reverse();
        GraphQLErrorPath { segments }
    }
}

#[derive(Clone, Debug)]
pub enum FlatValue<'a> {
    Null,
    Bool(bool),
    I64(i64),
    U64(u64),
    F64(f64),
    String(Cow<'a, str>),
    Object(ObjectId),
    List(ListId),
}

#[derive(Clone, Debug)]
pub struct FlatResponseData<'a> {
    root: ValueId,
    values: Vec<FlatValue<'a>>,
    objects: Vec<Vec<(Cow<'a, str>, ValueId)>>,
    object_symbols: Vec<Vec<(FieldSymbolId, ValueId)>>,
    lists: Vec<Vec<ValueId>>,
    field_symbols: HashMap<String, FieldSymbolId>,
    field_names: Vec<String>,
    symbol_generation: u64,
}

impl<'a> Default for FlatResponseData<'a> {
    fn default() -> Self {
        let mut data = Self {
            root: ValueId(0),
            values: Vec::new(),
            objects: Vec::new(),
            object_symbols: Vec::new(),
            lists: Vec::new(),
            field_symbols: HashMap::new(),
            field_names: Vec::new(),
            symbol_generation: 0,
        };
        data.root = data.push_value(FlatValue::Null);
        data
    }
}

impl<'a> FlatResponseData<'a> {
    pub fn empty_object() -> Self {
        let mut data = Self::default();
        let object_id = data.push_object(Vec::new());
        data.root = data.push_value(FlatValue::Object(object_id));
        data
    }

    pub fn is_null(&self) -> bool {
        matches!(self.values[self.root.0], FlatValue::Null)
    }

    pub fn root(&self) -> ValueId {
        self.root
    }

    pub fn root_object_fields(&self) -> Option<&[(Cow<'a, str>, ValueId)]> {
        match self.values.get(self.root.0) {
            Some(FlatValue::Object(obj_id)) => {
                self.objects.get(obj_id.0).map(|fields| fields.as_slice())
            }
            _ => None,
        }
    }

    pub fn value_kind(&self, id: ValueId) -> Option<&FlatValue<'a>> {
        self.values.get(id.0)
    }

    pub fn object_fields(&self, object_id: ObjectId) -> Option<&[(Cow<'a, str>, ValueId)]> {
        self.objects
            .get(object_id.0)
            .map(|fields| fields.as_slice())
    }

    pub fn list_items(&self, list_id: ListId) -> Option<&[ValueId]> {
        self.lists.get(list_id.0).map(|items| items.as_slice())
    }

    pub fn object_field(&self, value_id: ValueId, field_name: &str) -> Option<ValueId> {
        let FlatValue::Object(object_id) = self.values.get(value_id.0)? else {
            return None;
        };
        self.lookup_object_field(*object_id, field_name)
    }

    pub fn object_field_in_object_by_symbol(
        &self,
        object_id: ObjectId,
        field_symbol: FieldSymbolId,
    ) -> Option<ValueId> {
        self.lookup_object_field_by_symbol(object_id, field_symbol)
    }

    pub fn root_object_id(&self) -> Option<ObjectId> {
        match self.values.get(self.root.0) {
            Some(FlatValue::Object(object_id)) => Some(*object_id),
            _ => None,
        }
    }

    pub fn value_as_str(&self, value_id: ValueId) -> Option<&str> {
        match self.values.get(value_id.0) {
            Some(FlatValue::String(value)) => Some(value.as_ref()),
            _ => None,
        }
    }

    pub fn hash_value_by_requires<'i>(
        &self,
        value_id: ValueId,
        selection_set: &RuntimeCompiledSelectionSet,
        interner: &'i SchemaInterner,
        possible_types: &PossibleTypes,
        type_name_cache: &mut AHashMap<TypeId, &'i str>,
        typename_symbol: Option<FieldSymbolId>,
    ) -> u64 {
        let mut hasher = Xxh3::new();
        self.hash_value_with_requires(
            &mut hasher,
            value_id,
            selection_set,
            interner,
            possible_types,
            type_name_cache,
            typename_symbol,
        );
        hasher.finish()
    }

    pub fn set_root(&mut self, root: ValueId) {
        self.root = root;
    }

    pub fn clone_subtree_as_root(&self, root: ValueId) -> Self {
        let mut data = FlatResponseData::default();
        let new_root = data.clone_value_from_other(self, root);
        data.root = new_root;
        data
    }

    pub fn from_sonic_value_ref(value: ValueRef<'a>) -> Self {
        let mut data = FlatResponseData::default();
        let root = data.push_from_value_ref(value);
        data.root = root;
        data
    }

    pub fn add_value(&mut self, value: FlatValue<'a>) -> ValueId {
        self.push_value(value)
    }

    pub fn add_object(&mut self, object: Vec<(Cow<'a, str>, ValueId)>) -> ObjectId {
        self.push_object(object)
    }

    pub fn add_list(&mut self, list: Vec<ValueId>) -> ListId {
        self.push_list(list)
    }

    pub fn symbol_for(&self, field_name: &str) -> Option<FieldSymbolId> {
        self.field_symbols.get(field_name).copied()
    }

    pub fn field_name(&self, symbol: FieldSymbolId) -> Option<&str> {
        self.field_names.get(symbol.0 as usize).map(|s| s.as_str())
    }

    pub fn symbol_generation(&self) -> u64 {
        self.symbol_generation
    }

    pub fn merge_from(&mut self, source: &FlatResponseData<'a>) {
        self.deep_merge_values(self.root, source, source.root);
    }

    pub fn entities_value_ids(&self) -> Option<&[ValueId]> {
        let root_obj_id = match self.values.get(self.root.0) {
            Some(FlatValue::Object(obj_id)) => *obj_id,
            _ => return None,
        };

        let entities_symbol = self.symbol_for("_entities")?;
        let entities_id = self.lookup_object_field_by_symbol(root_obj_id, entities_symbol)?;

        let list = match self.values.get(entities_id.0) {
            Some(FlatValue::List(list_id)) => self.lists.get(list_id.0)?,
            _ => return None,
        };

        Some(list.as_slice())
    }

    pub fn collect_flatten_entities(
        &self,
        path: &[FlattenNodePathSegment],
        possible_types: &PossibleTypes,
    ) -> FlattenEntityTargets {
        let mut target_value_ids = Vec::new();
        let mut path_ids = Vec::new();
        let mut path_arena = PathArena::default();

        let mut stack: Vec<(ValueId, usize, Option<PathNodeId>)> = vec![(self.root, 0, None)];

        while let Some((current_id, path_index, path_node_id)) = stack.pop() {
            if path_index >= path.len() {
                if let Some(FlatValue::List(list_id)) = self.values.get(current_id.0) {
                    if let Some(items) = self.lists.get(list_id.0) {
                        for (index, item) in items.iter().enumerate() {
                            let path_id = path_arena.push_index(path_node_id, index);
                            target_value_ids.push(*item);
                            path_ids.push(path_id);
                        }
                    }
                } else if let Some(path_id) = path_node_id {
                    target_value_ids.push(current_id);
                    path_ids.push(path_id);
                }
                continue;
            }

            match &path[path_index] {
                FlattenNodePathSegment::List => {
                    if let Some(FlatValue::List(list_id)) = self.values.get(current_id.0) {
                        if let Some(items) = self.lists.get(list_id.0) {
                            for (index, item) in items.iter().enumerate().rev() {
                                let child_path = Some(path_arena.push_index(path_node_id, index));
                                stack.push((*item, path_index + 1, child_path));
                            }
                        }
                    }
                }
                FlattenNodePathSegment::Field(field_name) => {
                    if let Some(FlatValue::Object(object_id)) = self.values.get(current_id.0) {
                        if let Some(next_id) = self.lookup_object_field(*object_id, field_name) {
                            let child_path = Some(path_arena.push_field(path_node_id, field_name));
                            stack.push((next_id, path_index + 1, child_path));
                        }
                    }
                }
                FlattenNodePathSegment::TypeCondition(type_condition) => {
                    match self.values.get(current_id.0) {
                        Some(FlatValue::Object(object_id)) => {
                            let type_name = self
                                .lookup_object_field(*object_id, TYPENAME_FIELD_NAME)
                                .and_then(|id| {
                                    if let Some(FlatValue::String(v)) = self.values.get(id.0) {
                                        Some(v.as_ref())
                                    } else {
                                        None
                                    }
                                });
                            if type_name.is_none_or(|type_name| {
                                type_condition.iter().any(|condition| {
                                    possible_types
                                        .entity_satisfies_type_condition(type_name, condition)
                                })
                            }) {
                                stack.push((current_id, path_index + 1, path_node_id));
                            }
                        }
                        Some(FlatValue::List(list_id)) => {
                            if let Some(items) = self.lists.get(list_id.0) {
                                for (index, item) in items.iter().enumerate().rev() {
                                    let child_path =
                                        Some(path_arena.push_index(path_node_id, index));
                                    stack.push((*item, path_index, child_path));
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        FlattenEntityTargets {
            target_value_ids,
            path_ids,
            path_arena,
        }
    }

    pub fn merge_value_id_at_path(
        &mut self,
        path: &GraphQLErrorPath,
        source: &FlatResponseData<'a>,
        source_id: ValueId,
    ) -> bool {
        self.merge_value_id_at_segments(self.root, &path.segments, source, source_id)
    }

    pub fn apply_fetch_rewrite(
        &mut self,
        possible_types: &PossibleTypes,
        rewrite: &'a FetchRewrite,
    ) {
        match rewrite {
            FetchRewrite::KeyRenamer(key_renamer) => {
                let root = self.root;
                self.apply_key_renamer(
                    possible_types,
                    root,
                    key_renamer.path.as_slice(),
                    key_renamer,
                );
            }
            FetchRewrite::ValueSetter(value_setter) => {
                let root = self.root;
                self.apply_value_setter(
                    possible_types,
                    root,
                    value_setter.path.as_slice(),
                    value_setter,
                );
            }
        }
    }

    pub fn apply_fetch_rewrite_at_value(
        &mut self,
        possible_types: &PossibleTypes,
        value_id: ValueId,
        rewrite: &'a FetchRewrite,
    ) {
        match rewrite {
            FetchRewrite::KeyRenamer(key_renamer) => {
                self.apply_key_renamer(
                    possible_types,
                    value_id,
                    key_renamer.path.as_slice(),
                    key_renamer,
                );
            }
            FetchRewrite::ValueSetter(value_setter) => {
                self.apply_value_setter(
                    possible_types,
                    value_id,
                    value_setter.path.as_slice(),
                    value_setter,
                );
            }
        }
    }

    fn push_value(&mut self, value: FlatValue<'a>) -> ValueId {
        let id = ValueId(self.values.len());
        self.values.push(value);
        id
    }

    fn push_object(&mut self, object: Vec<(Cow<'a, str>, ValueId)>) -> ObjectId {
        let mut symbol_fields: Vec<(FieldSymbolId, ValueId)> = object
            .iter()
            .map(|(key, value)| (self.intern_field(key.as_ref()), *value))
            .collect();
        symbol_fields.sort_unstable_by_key(|(symbol, _)| symbol.0);

        self.push_object_with_symbols(object, symbol_fields)
    }

    fn push_object_with_symbols(
        &mut self,
        object: Vec<(Cow<'a, str>, ValueId)>,
        symbol_fields: Vec<(FieldSymbolId, ValueId)>,
    ) -> ObjectId {
        let id = ObjectId(self.objects.len());
        self.objects.push(object);
        self.object_symbols.push(symbol_fields);
        id
    }

    fn intern_field(&mut self, field_name: &str) -> FieldSymbolId {
        if let Some(symbol) = self.field_symbols.get(field_name) {
            return *symbol;
        }

        let symbol = FieldSymbolId(self.field_names.len() as u32);
        self.field_names.push(field_name.to_string());
        self.field_symbols.insert(field_name.to_string(), symbol);
        self.symbol_generation = self.symbol_generation.wrapping_add(1);
        symbol
    }

    fn lookup_object_field_by_symbol(
        &self,
        object_id: ObjectId,
        field_symbol: FieldSymbolId,
    ) -> Option<ValueId> {
        self.object_symbols.get(object_id.0).and_then(|object| {
            object
                .binary_search_by_key(&field_symbol.0, |(symbol, _)| symbol.0)
                .ok()
                .map(|index| object[index].1)
        })
    }

    fn insert_object_field_with_symbol(
        &mut self,
        object_id: ObjectId,
        key: Cow<'a, str>,
        value: ValueId,
        symbol: FieldSymbolId,
    ) {
        if let Some(object) = self.objects.get_mut(object_id.0) {
            object.push((key.clone(), value));
        }

        if let Some(symbols) = self.object_symbols.get_mut(object_id.0) {
            let pos = symbols
                .binary_search_by_key(&symbol.0, |(s, _)| s.0)
                .unwrap_or_else(|idx| idx);
            symbols.insert(pos, (symbol, value));
        }
    }

    fn push_list(&mut self, list: Vec<ValueId>) -> ListId {
        let id = ListId(self.lists.len());
        self.lists.push(list);
        id
    }

    fn push_from_value_ref(&mut self, value: ValueRef<'a>) -> ValueId {
        match value {
            ValueRef::Null => self.push_value(FlatValue::Null),
            ValueRef::Bool(v) => self.push_value(FlatValue::Bool(v)),
            ValueRef::Number(v) => {
                if let Some(v) = v.as_i64() {
                    self.push_value(FlatValue::I64(v))
                } else if let Some(v) = v.as_u64() {
                    self.push_value(FlatValue::U64(v))
                } else if let Some(v) = v.as_f64() {
                    self.push_value(FlatValue::F64(v))
                } else {
                    self.push_value(FlatValue::Null)
                }
            }
            ValueRef::String(v) => self.push_value(FlatValue::String(Cow::Borrowed(v))),
            ValueRef::Array(arr) => {
                let list = arr
                    .iter()
                    .map(|v| self.push_from_value_ref(v.as_ref()))
                    .collect::<Vec<_>>();
                let list_id = self.push_list(list);
                self.push_value(FlatValue::List(list_id))
            }
            ValueRef::Object(map) => {
                let mut obj = map
                    .iter()
                    .map(|(k, v)| (Cow::Borrowed(k), self.push_from_value_ref(v.as_ref())))
                    .collect::<Vec<_>>();
                obj.sort_unstable_by(|(left, _), (right, _)| left.as_ref().cmp(right.as_ref()));
                let obj_id = self.push_object(obj);
                self.push_value(FlatValue::Object(obj_id))
            }
        }
    }

    fn clone_value_from_other(&mut self, other: &FlatResponseData<'a>, id: ValueId) -> ValueId {
        let mut symbol_remap_cache = AHashMap::new();
        let value = self.clone_flat_value_from_other(other, id, &mut symbol_remap_cache);
        self.push_value(value)
    }

    fn clone_flat_value_from_other(
        &mut self,
        other: &FlatResponseData<'a>,
        id: ValueId,
        symbol_remap_cache: &mut AHashMap<FieldSymbolId, FieldSymbolId>,
    ) -> FlatValue<'a> {
        match other.values.get(id.0) {
            Some(FlatValue::Null) | None => FlatValue::Null,
            Some(FlatValue::Bool(v)) => FlatValue::Bool(*v),
            Some(FlatValue::I64(v)) => FlatValue::I64(*v),
            Some(FlatValue::U64(v)) => FlatValue::U64(*v),
            Some(FlatValue::F64(v)) => FlatValue::F64(*v),
            Some(FlatValue::String(v)) => FlatValue::String(v.clone()),
            Some(FlatValue::List(list_id)) => {
                let source_items = other.lists.get(list_id.0).map(Vec::as_slice).unwrap_or(&[]);
                let mut list = Vec::with_capacity(source_items.len());
                for &item in source_items {
                    let value = self.clone_flat_value_from_other(other, item, symbol_remap_cache);
                    let value_id = self.push_value(value);
                    list.push(value_id);
                }
                let new_list_id = self.push_list(list);
                FlatValue::List(new_list_id)
            }
            Some(FlatValue::Object(obj_id)) => {
                let source_fields = other
                    .objects
                    .get(obj_id.0)
                    .map(Vec::as_slice)
                    .unwrap_or(&[]);
                let mut obj = Vec::with_capacity(source_fields.len());
                let mut value_by_field = AHashMap::with_capacity(source_fields.len());
                for (k, v) in source_fields {
                    let value = self.clone_flat_value_from_other(other, *v, symbol_remap_cache);
                    let value_id = self.push_value(value);
                    value_by_field.insert(k.as_ref(), value_id);
                    obj.push((k.clone(), value_id));
                }
                let source_symbol_fields = other
                    .object_symbols
                    .get(obj_id.0)
                    .map(Vec::as_slice)
                    .unwrap_or(&[]);
                let mut symbol_fields = Vec::with_capacity(source_symbol_fields.len());
                for &(source_symbol, _) in source_symbol_fields {
                    let source_name = other.field_name(source_symbol).unwrap_or_default();
                    let target_symbol = *symbol_remap_cache
                        .entry(source_symbol)
                        .or_insert_with(|| self.intern_field(source_name));
                    if let Some(&target_value_id) = value_by_field.get(source_name) {
                        symbol_fields.push((target_symbol, target_value_id));
                    }
                }
                symbol_fields.sort_unstable_by_key(|(symbol, _)| symbol.0);
                let new_obj_id = self.push_object_with_symbols(obj, symbol_fields);
                FlatValue::Object(new_obj_id)
            }
        }
    }

    pub fn deep_merge_values(
        &mut self,
        target_id: ValueId,
        source: &FlatResponseData<'a>,
        source_id: ValueId,
    ) {
        let source_kind = match source.values.get(source_id.0) {
            Some(FlatValue::Null) | None => return,
            Some(FlatValue::Object(object_id)) => (Some(*object_id), None),
            Some(FlatValue::List(list_id)) => (None, Some(*list_id)),
            Some(_) => (None, None),
        };

        let target_kind = match self.values.get(target_id.0) {
            Some(FlatValue::Object(object_id)) => (Some(*object_id), None),
            Some(FlatValue::List(list_id)) => (None, Some(*list_id)),
            _ => (None, None),
        };

        match (target_kind, source_kind) {
            ((Some(target_obj_id), _), (Some(source_obj_id), _)) => {
                let source_fields = source
                    .object_symbols
                    .get(source_obj_id.0)
                    .map(Vec::as_slice)
                    .unwrap_or_default();
                let mut symbol_remap_cache = AHashMap::with_capacity(source_fields.len());

                for &(source_symbol, source_field_id) in source_fields {
                    let target_symbol =
                        *symbol_remap_cache.entry(source_symbol).or_insert_with(|| {
                            let source_name = source.field_name(source_symbol).unwrap_or_default();
                            self.intern_field(source_name)
                        });
                    let existing = self.lookup_object_field_by_symbol(target_obj_id, target_symbol);

                    if let Some(existing_id) = existing {
                        self.deep_merge_values(existing_id, source, source_field_id);
                    } else {
                        let source_name = source.field_name(source_symbol).unwrap_or_default();
                        let cloned = self.clone_value_from_other(source, source_field_id);
                        self.insert_object_field_with_symbol(
                            target_obj_id,
                            Cow::Owned(source_name.to_string()),
                            cloned,
                            target_symbol,
                        );
                    }
                }
            }
            ((_, Some(target_list_id)), (_, Some(source_list_id))) => {
                let source_items = source
                    .lists
                    .get(source_list_id.0)
                    .map(Vec::as_slice)
                    .unwrap_or_default();
                let target_len = self.lists.get(target_list_id.0).map_or(0, Vec::len);
                let merge_len = source_items.len().min(target_len);
                for idx in 0..merge_len {
                    let Some(target_item) = self
                        .lists
                        .get(target_list_id.0)
                        .and_then(|items| items.get(idx))
                        .copied()
                    else {
                        continue;
                    };
                    let source_item = source_items[idx];
                    self.deep_merge_values(target_item, source, source_item);
                }
            }
            _ => {
                let mut symbol_remap_cache = AHashMap::new();
                self.values[target_id.0] =
                    self.clone_flat_value_from_other(source, source_id, &mut symbol_remap_cache);
            }
        }
    }

    fn hash_value_with_requires<'i, H: Hasher>(
        &self,
        state: &mut H,
        value_id: ValueId,
        selection_set: &RuntimeCompiledSelectionSet,
        interner: &'i SchemaInterner,
        possible_types: &PossibleTypes,
        type_name_cache: &mut AHashMap<TypeId, &'i str>,
        typename_symbol: Option<FieldSymbolId>,
    ) {
        if selection_set.items.is_empty() {
            self.hash_full_value(state, value_id);
            return;
        }

        match self.values.get(value_id.0) {
            Some(FlatValue::Object(object_id)) => {
                self.hash_object_with_requires(
                    state,
                    *object_id,
                    selection_set,
                    interner,
                    possible_types,
                    type_name_cache,
                    typename_symbol,
                );
            }
            Some(FlatValue::List(list_id)) => {
                if let Some(items) = self.lists.get(list_id.0) {
                    for item in items {
                        self.hash_value_with_requires(
                            state,
                            *item,
                            selection_set,
                            interner,
                            possible_types,
                            type_name_cache,
                            typename_symbol,
                        );
                    }
                }
            }
            _ => self.hash_full_value(state, value_id),
        }
    }

    fn hash_object_with_requires<'i, H: Hasher>(
        &self,
        state: &mut H,
        object_id: ObjectId,
        selection_set: &RuntimeCompiledSelectionSet,
        interner: &'i SchemaInterner,
        possible_types: &PossibleTypes,
        type_name_cache: &mut AHashMap<TypeId, &'i str>,
        typename_symbol: Option<FieldSymbolId>,
    ) {
        let object_type_name = typename_symbol
            .and_then(|symbol| self.lookup_object_field_by_symbol(object_id, symbol))
            .and_then(|id| self.value_as_str(id));

        for item in &selection_set.items {
            match item {
                RuntimeCompiledSelectionItem::Field(field_selection) => {
                    if let Some(symbol) = field_selection.symbol {
                        if let Some(value_id) =
                            self.lookup_object_field_by_symbol(object_id, symbol)
                        {
                            symbol.0.hash(state);
                            self.hash_value_with_requires(
                                state,
                                value_id,
                                &field_selection.selections,
                                interner,
                                possible_types,
                                type_name_cache,
                                typename_symbol,
                            );
                        }
                    }
                }
                RuntimeCompiledSelectionItem::InlineFragment(inline_fragment) => {
                    let type_condition = *type_name_cache
                        .entry(inline_fragment.type_condition)
                        .or_insert_with(|| interner.resolve_type(&inline_fragment.type_condition));
                    let type_name = object_type_name.unwrap_or(type_condition);

                    if possible_types.entity_satisfies_type_condition(type_name, type_condition) {
                        self.hash_object_with_requires(
                            state,
                            object_id,
                            &inline_fragment.selections,
                            interner,
                            possible_types,
                            type_name_cache,
                            typename_symbol,
                        );
                    }
                }
            }
        }
    }

    fn hash_full_value<H: Hasher>(&self, state: &mut H, value_id: ValueId) {
        match self.values.get(value_id.0) {
            Some(FlatValue::Null) | None => {
                0u8.hash(state);
            }
            Some(FlatValue::Bool(v)) => {
                1u8.hash(state);
                v.hash(state);
            }
            Some(FlatValue::I64(v)) => {
                2u8.hash(state);
                v.hash(state);
            }
            Some(FlatValue::U64(v)) => {
                3u8.hash(state);
                v.hash(state);
            }
            Some(FlatValue::F64(v)) => {
                4u8.hash(state);
                v.to_bits().hash(state);
            }
            Some(FlatValue::String(v)) => {
                5u8.hash(state);
                v.as_ref().hash(state);
            }
            Some(FlatValue::Object(object_id)) => {
                6u8.hash(state);
                if let Some(fields) = self.object_symbols.get(object_id.0) {
                    fields.len().hash(state);
                    for (symbol, child_id) in fields {
                        symbol.0.hash(state);
                        self.hash_full_value(state, *child_id);
                    }
                }
            }
            Some(FlatValue::List(list_id)) => {
                7u8.hash(state);
                if let Some(items) = self.lists.get(list_id.0) {
                    items.len().hash(state);
                    for child_id in items {
                        self.hash_full_value(state, *child_id);
                    }
                }
            }
        }
    }

    fn merge_value_id_at_segments(
        &mut self,
        current_id: ValueId,
        segments: &[GraphQLErrorPathSegment],
        source: &FlatResponseData<'a>,
        source_id: ValueId,
    ) -> bool {
        if segments.is_empty() {
            self.deep_merge_values(current_id, source, source_id);
            return true;
        }

        match (self.values.get(current_id.0).cloned(), &segments[0]) {
            (Some(FlatValue::Object(object_id)), GraphQLErrorPathSegment::String(field)) => {
                if let Some(next_id) = self.lookup_object_field(object_id, field) {
                    self.merge_value_id_at_segments(next_id, &segments[1..], source, source_id)
                } else {
                    false
                }
            }
            (Some(FlatValue::List(list_id)), GraphQLErrorPathSegment::Index(index)) => {
                let next = self
                    .lists
                    .get(list_id.0)
                    .and_then(|list| list.get(*index))
                    .copied();
                if let Some(next_id) = next {
                    self.merge_value_id_at_segments(next_id, &segments[1..], source, source_id)
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    fn apply_key_renamer(
        &mut self,
        possible_types: &PossibleTypes,
        value_id: ValueId,
        path: &[FetchNodePathSegment],
        key_renamer: &'a KeyRenamer,
    ) {
        if path.is_empty() {
            return;
        }

        let mut stack = vec![(value_id, 0usize)];
        while let Some((current_value_id, path_index)) = stack.pop() {
            if path_index >= path.len() {
                continue;
            }

            let current_kind = match self.values.get(current_value_id.0) {
                Some(FlatValue::List(list_id)) => Some((Some(*list_id), None)),
                Some(FlatValue::Object(object_id)) => Some((None, Some(*object_id))),
                _ => None,
            };

            match current_kind {
                Some((Some(list_id), None)) => {
                    if let Some(items) = self.lists.get(list_id.0) {
                        for &item in items {
                            stack.push((item, path_index));
                        }
                    }
                }
                Some((None, Some(object_id))) => match &path[path_index] {
                    FetchNodePathSegment::TypenameEquals(type_condition) => {
                        if self.object_satisfies_type_condition(
                            possible_types,
                            object_id,
                            type_condition,
                        ) {
                            stack.push((current_value_id, path_index + 1));
                        }
                    }
                    FetchNodePathSegment::Key(field_name) => {
                        if path_index + 1 >= path.len() {
                            if field_name != &key_renamer.rename_key_to {
                                let old_symbol = self.symbol_for(field_name);
                                let new_symbol =
                                    self.intern_field(key_renamer.rename_key_to.as_str());

                                if let Some(object) = self.objects.get_mut(object_id.0) {
                                    if let Some((key, _)) = object
                                        .iter_mut()
                                        .find(|(key, _)| key.as_ref() == field_name)
                                    {
                                        *key = Cow::Borrowed(key_renamer.rename_key_to.as_str());
                                    }
                                }

                                if let (Some(symbols), Some(old_symbol)) =
                                    (self.object_symbols.get_mut(object_id.0), old_symbol)
                                {
                                    if let Ok(index) = symbols
                                        .binary_search_by_key(&old_symbol.0, |(symbol, _)| symbol.0)
                                    {
                                        let (_, child_value_id) = symbols.remove(index);
                                        let insert_at = symbols
                                            .binary_search_by_key(&new_symbol.0, |(symbol, _)| {
                                                symbol.0
                                            })
                                            .unwrap_or_else(|idx| idx);
                                        symbols.insert(insert_at, (new_symbol, child_value_id));
                                    }
                                }
                            }
                        } else if let Some(next_id) =
                            self.lookup_object_field(object_id, field_name)
                        {
                            stack.push((next_id, path_index + 1));
                        }
                    }
                },
                _ => {}
            }
        }
    }

    fn apply_value_setter(
        &mut self,
        possible_types: &PossibleTypes,
        value_id: ValueId,
        path: &[FetchNodePathSegment],
        value_setter: &'a ValueSetter,
    ) {
        let mut stack = vec![(value_id, 0usize)];
        while let Some((current_value_id, path_index)) = stack.pop() {
            if path_index >= path.len() {
                self.values[current_value_id.0] =
                    FlatValue::String(Cow::Borrowed(value_setter.set_value_to.as_str()));
                continue;
            }

            let current_kind = match self.values.get(current_value_id.0) {
                Some(FlatValue::List(list_id)) => Some((Some(*list_id), None)),
                Some(FlatValue::Object(object_id)) => Some((None, Some(*object_id))),
                _ => None,
            };

            match current_kind {
                Some((Some(list_id), None)) => {
                    if let Some(items) = self.lists.get(list_id.0) {
                        for &item in items {
                            stack.push((item, path_index));
                        }
                    }
                }
                Some((None, Some(object_id))) => match &path[path_index] {
                    FetchNodePathSegment::TypenameEquals(type_condition) => {
                        if self.object_satisfies_type_condition(
                            possible_types,
                            object_id,
                            type_condition,
                        ) {
                            stack.push((current_value_id, path_index + 1));
                        }
                    }
                    FetchNodePathSegment::Key(field_name) => {
                        if let Some(next_id) = self.lookup_object_field(object_id, field_name) {
                            stack.push((next_id, path_index + 1));
                        }
                    }
                },
                _ => {}
            }
        }
    }

    fn object_satisfies_type_condition(
        &self,
        possible_types: &PossibleTypes,
        object_id: ObjectId,
        type_condition: &std::collections::BTreeSet<String>,
    ) -> bool {
        let maybe_type_name = self
            .lookup_object_field(object_id, TYPENAME_FIELD_NAME)
            .and_then(|id| {
                if let Some(FlatValue::String(value)) = self.values.get(id.0) {
                    Some(value.as_ref())
                } else {
                    None
                }
            });

        maybe_type_name.is_none_or(|type_name| {
            type_condition.iter().any(|condition| {
                possible_types.entity_satisfies_type_condition(type_name, condition)
            })
        })
    }

    fn lookup_object_field(&self, object_id: ObjectId, field_name: &str) -> Option<ValueId> {
        let field_symbol = self.symbol_for(field_name)?;
        self.lookup_object_field_by_symbol(object_id, field_symbol)
    }
}

pub struct FlatValueSeed<'a, 'store> {
    store: &'store mut FlatResponseData<'a>,
    schema_interner: Option<&'store SchemaInterner>,
}

impl<'a, 'store> FlatValueSeed<'a, 'store> {
    pub fn new(store: &'store mut FlatResponseData<'a>) -> Self {
        Self {
            store,
            schema_interner: None,
        }
    }

    pub fn with_schema_interner(
        store: &'store mut FlatResponseData<'a>,
        schema_interner: &'store SchemaInterner,
    ) -> Self {
        Self {
            store,
            schema_interner: Some(schema_interner),
        }
    }
}

impl<'de, 'a, 'store> DeserializeSeed<'de> for FlatValueSeed<'a, 'store>
where
    'de: 'a,
{
    type Value = ValueId;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(FlatValueVisitor {
            store: self.store,
            schema_interner: self.schema_interner,
        })
    }
}

struct FlatValueVisitor<'a, 'store> {
    store: &'store mut FlatResponseData<'a>,
    schema_interner: Option<&'store SchemaInterner>,
}

impl<'de, 'a, 'store> Visitor<'de> for FlatValueVisitor<'a, 'store>
where
    'de: 'a,
{
    type Value = ValueId;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("any valid JSON value")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(self.store.push_value(FlatValue::Bool(value)))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(self.store.push_value(FlatValue::I64(value)))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(self.store.push_value(FlatValue::U64(value)))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E> {
        Ok(self.store.push_value(FlatValue::F64(value)))
    }

    fn visit_borrowed_str<E>(self, value: &'de str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(self
            .store
            .push_value(FlatValue::String(Cow::Borrowed(value))))
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(self
            .store
            .push_value(FlatValue::String(Cow::Owned(v.to_owned()))))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(self.store.push_value(FlatValue::String(Cow::Owned(value))))
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(self.store.push_value(FlatValue::Null))
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut elements = Vec::with_capacity(seq.size_hint().unwrap_or(0));
        while let Some(elem) = seq.next_element_seed(FlatValueSeed {
            store: self.store,
            schema_interner: self.schema_interner,
        })? {
            elements.push(elem);
        }

        let list_id = self.store.push_list(elements);
        Ok(self.store.push_value(FlatValue::List(list_id)))
    }

    fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
    where
        M: MapAccess<'de>,
    {
        let mut entries: Vec<(Cow<'a, str>, ValueId)> =
            Vec::with_capacity(map.size_hint().unwrap_or(0));
        while let Some(key) = map.next_key::<&'de str>()? {
            if let Some(schema_interner) = self.schema_interner {
                if schema_interner.get_field(key).is_none() {
                    return Err(de::Error::custom(format!(
                        "unknown key in data payload: {key}"
                    )));
                }
            }
            let value_id = map.next_value_seed(FlatValueSeed {
                store: self.store,
                schema_interner: self.schema_interner,
            })?;
            entries.push((Cow::Borrowed(key), value_id));
        }
        entries.sort_unstable_by(|(left, _), (right, _)| left.as_ref().cmp(right.as_ref()));

        let object_id = self.store.push_object(entries);
        Ok(self.store.push_value(FlatValue::Object(object_id)))
    }
}
