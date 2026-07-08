use core::fmt;
use std::borrow::Cow;
use std::cell::RefCell;
use std::sync::Arc;

use bytes::Bytes;
use serde::{
    de::{self, DeserializeSeed, Deserializer, MapAccess, SeqAccess, Visitor},
    Deserialize,
};

use crate::executors::error::SubgraphExecutorError;
use crate::introspection::schema::FieldNullability;
use crate::response::flat_plan::{
    FetchWritePlan, FieldWritePlan, ListWritePlan, ObjectWritePlan, ValueWritePlan, WriteResult,
};
use crate::response::flat_store::{
    FlatObjectField, FlatResponseStore, FlatValue, FlatValueId, ResponseKeys,
};
use crate::response::graphql_error::GraphQLError;
use crate::response::subgraph_response::SubgraphResponse;
use crate::response::value::Value;

/// A response part produced by decoding a subgraph response through a fetch write plan.
#[derive(Debug, Default)]
pub struct FlatResponsePart {
    pub store: FlatResponseStore<'static>,
    pub keys: Arc<ResponseKeys>,
    pub data_root: Option<FlatValueId>,
    pub errors: Option<Vec<GraphQLError>>,
    pub bytes: Option<Bytes>,
    pub propagated_null: bool,
}

impl FlatResponsePart {
    pub fn empty(propagated_null: bool) -> Self {
        Self {
            store: FlatResponseStore::new(),
            keys: Arc::new(ResponseKeys::default()),
            data_root: None,
            errors: None,
            bytes: None,
            propagated_null,
        }
    }
}

/// Deserialize subgraph HTTP response bytes into a FlatResponsePart using a compiled fetch plan.
pub fn deserialize_fetch_into_part(
    bytes: Bytes,
    plan: &FetchWritePlan,
) -> Result<FlatResponsePart, SubgraphExecutorError> {
    let bytes_ref: &[u8] = &bytes;
    let bytes_ref: &'static [u8] = unsafe { std::mem::transmute(bytes_ref) };
    let mut deserializer = sonic_rs::Deserializer::from_slice(bytes_ref);

    let seed = FusedFetchSeed { plan };
    let mut part: FlatResponsePart = seed
        .deserialize(&mut deserializer)
        .map_err(|e| SubgraphExecutorError::ResponseDeserializationFailure(e, None))?;

    deserializer
        .end()
        .map_err(|e| SubgraphExecutorError::ResponseDeserializationFailure(e, None))?;

    part.store.retain_bytes(bytes.clone());
    part.bytes = Some(bytes);
    Ok(part)
}

/// Deserialize subgraph HTTP response bytes through the fused pipeline,
/// then serialize into final JSON response bytes.
pub fn deserialize_and_serialize_fused(
    bytes: Bytes,
    plan: &FetchWritePlan,
) -> Result<Vec<u8>, SubgraphExecutorError> {
    let part = deserialize_fetch_into_part(bytes, plan)?;
    let root = part.data_root.unwrap_or(part.data_root.unwrap());
    let mut buf = Vec::with_capacity(1024);
    crate::response::flat_store::serialize_store_data(&part.store, &part.keys, root, &mut buf);
    Ok(buf)
}

struct FusedFetchSeed<'a> {
    plan: &'a FetchWritePlan,
}

impl<'a, 'de> DeserializeSeed<'de> for FusedFetchSeed<'a> {
    type Value = FlatResponsePart;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_map(FusedFetchVisitor { plan: self.plan })
    }
}

struct FusedFetchVisitor<'a> {
    plan: &'a FetchWritePlan,
}

impl<'a, 'de> Visitor<'de> for FusedFetchVisitor<'a> {
    type Value = FlatResponsePart;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a GraphQL response object")
    }

    fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
    where
        M: MapAccess<'de>,
    {
        let mut store: FlatResponseStore = FlatResponseStore::new();
        let keys: Arc<ResponseKeys> = Arc::clone(&self.plan.keys);
        let mut errors: Option<Vec<GraphQLError>> = None;
        let mut data_root: Option<FlatValueId> = None;
        let mut propagated_null = false;

        while let Some(key) = map.next_key::<&str>()? {
            match key {
                "data" => {
                    if data_root.is_some() {
                        return Err(de::Error::duplicate_field("data"));
                    }
                    let write_ctx = FusedWriteCtx {
                        store: RefCell::new(&mut store),
                        field_values_pool: RefCell::new(Vec::new()),
                    };
                    let result = map
                        .next_value_seed(FusedValueSeed {
                            plan: &self.plan.data,
                            ctx: &write_ctx,
                        })
                        .map_err(|e| de::Error::custom(format!("data: {e}")))?;

                    if !result.propagated_null {
                        data_root = Some(result.value_id);
                    } else {
                        propagated_null = true;
                    }
                }
                "errors" => {
                    if errors.is_some() {
                        return Err(de::Error::duplicate_field("errors"));
                    }
                    errors = Some(map.next_value()?);
                }
                "extensions" => {
                    let _: de::IgnoredAny = map.next_value()?;
                }
                _ => {
                    let _ = map.next_value::<de::IgnoredAny>()?;
                }
            }
        }

        Ok(FlatResponsePart {
            store,
            keys,
            data_root,
            errors,
            bytes: None,
            propagated_null,
        })
    }
}

/// Mutable context passed through the deserialization tree.
pub struct FusedWriteCtx<'a> {
    pub store: RefCell<&'a mut FlatResponseStore<'static>>,
    pub field_values_pool: RefCell<Vec<Vec<Option<WriteResult>>>>,
}

#[derive(Clone, Copy)]
struct FusedValueSeed<'a, 'plan, 'ctx> {
    plan: &'plan ValueWritePlan,
    ctx: &'ctx FusedWriteCtx<'a>,
}

impl<'de> DeserializeSeed<'de> for FusedValueSeed<'_, '_, '_> {
    type Value = WriteResult;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        match self.plan {
            ValueWritePlan::Leaf(leaf) => {
                if leaf.custom_scalar {
                    let raw = sonic_rs::LazyValue::deserialize(deserializer)?;
                    let raw_cow = raw.as_raw_cow();
                    let raw_cow: Cow<'static, str> = match raw_cow {
                        Cow::Borrowed(s) => {
                            // SAFETY: Response bytes are transmuted to 'static in deserialize_fetch_into_part
                            // and retained in FlatResponsePart.bytes.
                            Cow::Borrowed(unsafe { core::mem::transmute::<&str, &'static str>(s) })
                        }
                        Cow::Owned(s) => Cow::Owned(s),
                    };
                    Ok(WriteResult::ok(
                        self.ctx
                            .store
                            .borrow_mut()
                            .alloc_raw_json(raw_cow),
                    ))
                } else {
                    deserializer.deserialize_any(FusedLeafVisitor {
                        ctx: self.ctx,
                        nullable: !leaf.nullability.is_non_null(),
                    })
                }
            }
            ValueWritePlan::Object(obj) => deserializer.deserialize_any(FusedObjectVisitor {
                plan: obj,
                ctx: self.ctx,
            }),
            ValueWritePlan::List(list) => deserializer.deserialize_any(FusedListVisitor {
                plan: list,
                ctx: self.ctx,
            }),
            ValueWritePlan::Skip => {
                let _ = de::IgnoredAny::deserialize(deserializer)?;
                Ok(WriteResult::ok(self.ctx.store.borrow_mut().alloc_null()))
            }
        }
    }
}

struct FusedObjectVisitor<'a, 'plan, 'ctx> {
    plan: &'plan ObjectWritePlan,
    ctx: &'ctx FusedWriteCtx<'a>,
}

impl<'de> Visitor<'de> for FusedObjectVisitor<'_, '_, '_> {
    type Value = WriteResult;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a JSON object")
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        if self.plan.nullability.is_non_null() {
            Ok(WriteResult::propagate_null())
        } else {
            Ok(WriteResult::ok(self.ctx.store.borrow_mut().alloc_null()))
        }
    }

    fn visit_none<E>(self) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.visit_unit()
    }

    fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
    where
        M: MapAccess<'de>,
    {
        let obj_id = deserialize_shaped_object(self.plan, self.ctx, &mut map)?;
        Ok(WriteResult::ok(obj_id))
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        // When the plan expects an object but the JSON is an array, treat each
        // element as an instance of the object plan (list of objects).
        let mut builder = self
            .ctx
            .store
            .borrow_mut()
            .reserve_list(seq.size_hint().unwrap_or(0));

        while let Some(elem) =
            seq.next_element_seed(FusedObjectSeqSeed {
                plan: self.plan,
                ctx: self.ctx,
            })?
        {
            builder.push(elem);
        }

        let list_id = self.ctx.store.borrow_mut().finish_list(builder);
        Ok(WriteResult::ok(list_id))
    }
}

struct FusedObjectSeqSeed<'a, 'plan, 'ctx> {
    plan: &'plan ObjectWritePlan,
    ctx: &'ctx FusedWriteCtx<'a>,
}

impl<'de> DeserializeSeed<'de> for FusedObjectSeqSeed<'_, '_, '_> {
    type Value = FlatValueId;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(FusedObjectSeqVisitor {
            plan: self.plan,
            ctx: self.ctx,
        })
    }
}

struct FusedObjectSeqVisitor<'a, 'plan, 'ctx> {
    plan: &'plan ObjectWritePlan,
    ctx: &'ctx FusedWriteCtx<'a>,
}

impl<'de> Visitor<'de> for FusedObjectSeqVisitor<'_, '_, '_> {
    type Value = FlatValueId;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a JSON object")
    }

    fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
    where
        M: MapAccess<'de>,
    {
        deserialize_shaped_object(self.plan, self.ctx, &mut map)
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(self.ctx.store.borrow_mut().alloc_null())
    }
}

fn deserialize_shaped_object<'a, 'ctx, 'de, M>(
    plan: &ObjectWritePlan,
    ctx: &'ctx FusedWriteCtx<'a>,
    map: &mut M,
) -> Result<FlatValueId, M::Error>
where
    M: MapAccess<'de>,
{
    let field_count = plan.fields.len();
    let mut field_values = ctx.field_values_pool.borrow_mut().pop().unwrap_or_default();
    field_values.resize(field_count, None);
    let mut offset = 0usize;

    while let Some(key) = map.next_key::<&str>()? {
        if let Some(index) = find_field_index(&plan.fields, key, &mut offset) {
            if field_values[index].is_none() {
                let result = map
                    .next_value_seed(FusedValueSeed {
                        plan: &plan.fields[index].value,
                        ctx,
                    })
                    .map_err(|e| de::Error::custom(format!("field {key}: {e}")))?;

                if result.propagated_null && plan.fields[index].nullability.is_non_null() {
                    return Err(de::Error::custom(format!(
                        "null value for non-null field '{key}'"
                    )));
                }
                field_values[index] = Some(result);
            } else {
                let _ = map.next_value::<de::IgnoredAny>()?;
            }
        } else {
            let _ = map.next_value::<de::IgnoredAny>()?;
        }
    }

    let mut store = ctx.store.borrow_mut();
    let mut obj_builder = store.begin_object_fields(field_count);

    for (i, maybe_result) in field_values.iter_mut().enumerate() {
        let key_id = plan.fields[i].response_key_id;
        let value_id = match maybe_result.take() {
            Some(result) if result.propagated_null => {
                if plan.fields[i].nullability.is_non_null() {
                    return Err(de::Error::custom(format!(
                        "null value for non-null field '{}'",
                        plan.fields[i].response_key
                    )));
                }
                store.alloc_null()
            }
            Some(result) => result.value_id,
            None => {
                if plan.fields[i].nullability.is_non_null() {
                    return Err(de::Error::custom(format!(
                        "missing non-null field '{}'",
                        plan.fields[i].response_key
                    )));
                }
                store.alloc_null()
            }
        };
        store.push_object_field(&mut obj_builder, FlatObjectField {
            response_key: key_id,
            value: value_id,
        });
    }

    let obj_id = store.finish_object_fields(obj_builder);
    drop(store);

    field_values.clear();
    ctx.field_values_pool.borrow_mut().push(field_values);

    Ok(obj_id)
}

struct FusedListVisitor<'a, 'plan, 'ctx> {
    plan: &'plan ListWritePlan,
    ctx: &'ctx FusedWriteCtx<'a>,
}

impl<'de> Visitor<'de> for FusedListVisitor<'_, '_, '_> {
    type Value = WriteResult;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a JSON array")
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        if self.plan.nullability.is_non_null() {
            Ok(WriteResult::propagate_null())
        } else {
            Ok(WriteResult::ok(self.ctx.store.borrow_mut().alloc_null()))
        }
    }

    fn visit_none<E>(self) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.visit_unit()
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut builder = self.ctx.store.borrow_mut().reserve_list(seq.size_hint().unwrap_or(0));
        let item_non_null = self
            .plan
            .nullability
            .list_item()
            .is_some_and(FieldNullability::is_non_null);

        while let Some(result) = seq.next_element_seed(FusedValueSeed {
            plan: &self.plan.item,
            ctx: self.ctx,
        })? {
            if result.propagated_null {
                if item_non_null {
                    return Ok(WriteResult::propagate_null());
                }
                builder.push(self.ctx.store.borrow_mut().alloc_null());
            } else {
                builder.push(result.value_id);
            }
        }

        let list_id = self.ctx.store.borrow_mut().finish_list(builder);
        Ok(WriteResult::ok(list_id))
    }
}

#[derive(Clone, Copy)]
struct FusedLeafSeqSeed<'a, 'ctx> {
    ctx: &'ctx FusedWriteCtx<'a>,
}

impl<'de> DeserializeSeed<'de> for FusedLeafSeqSeed<'_, '_> {
    type Value = FlatValueId;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        let result = deserializer.deserialize_any(FusedLeafVisitor {
            ctx: self.ctx,
            nullable: true,
        })?;
        Ok(result.value_id)
    }
}

struct FusedLeafVisitor<'a, 'ctx> {
    ctx: &'ctx FusedWriteCtx<'a>,
    nullable: bool,
}

impl<'de> Visitor<'de> for FusedLeafVisitor<'_, '_> {
    type Value = WriteResult;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a JSON scalar value")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(WriteResult::ok(self.ctx.store.borrow_mut().alloc_bool(value)))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(WriteResult::ok(self.ctx.store.borrow_mut().alloc_i64(value)))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(WriteResult::ok(self.ctx.store.borrow_mut().alloc_u64(value)))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E> {
        Ok(WriteResult::ok(self.ctx.store.borrow_mut().alloc_f64(value)))
    }

    fn visit_borrowed_str<E>(self, value: &'de str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        // SAFETY: Response bytes are transmuted to 'static in deserialize_fetch_into_part
        // and retained in FlatResponsePart.bytes.
        let value: &'static str = unsafe { core::mem::transmute::<&str, &'static str>(value) };
        Ok(WriteResult::ok(
            self.ctx.store.borrow_mut().alloc_string(Cow::Borrowed(value)),
        ))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(WriteResult::ok(
            self.ctx.store.borrow_mut().alloc_string(Cow::Owned(value.to_owned())),
        ))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(WriteResult::ok(
            self.ctx.store.borrow_mut().alloc_string(Cow::Owned(value)),
        ))
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        if self.nullable {
            Ok(WriteResult::ok(self.ctx.store.borrow_mut().alloc_null()))
        } else {
            Ok(WriteResult::propagate_null())
        }
    }

    fn visit_none<E>(self) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.visit_unit()
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut builder = self
            .ctx
            .store
            .borrow_mut()
            .reserve_list(seq.size_hint().unwrap_or(0));

        while let Some(elem) = seq.next_element_seed(FusedLeafSeqSeed {
            ctx: self.ctx,
        })? {
            builder.push(elem);
        }

        let list_id = self.ctx.store.borrow_mut().finish_list(builder);
        Ok(WriteResult::ok(list_id))
    }
}

fn find_field_index(
    fields: &[FieldWritePlan],
    key: &str,
    offset: &mut usize,
) -> Option<usize> {
    let len = fields.len();
    if len == 0 {
        return None;
    }

    let start = (*offset).min(len);
    if start == len {
        return None;
    }

    if fields[start].source_key.as_ref() == key {
        *offset = start + 1;
        return Some(start);
    }

    for index in (start + 1)..len {
        if fields[index].source_key.as_ref() == key {
            return Some(index);
        }
    }

    None
}

/// Fallback: convert a flat response part into a SubgraphResponse.
/// Useful during migration when the execution pipeline still expects SubgraphResponse.
pub fn flat_part_to_subgraph_response(
    part: FlatResponsePart,
) -> Result<SubgraphResponse<'static>, SubgraphExecutorError> {
    let data = if part.propagated_null {
        Value::Null
    } else {
        // Serialize the flat store data back into a Value::Object
        // This is temporary until the pipeline fully supports flat responses.
        flat_store_to_value(&part.store, part.data_root.unwrap_or(part.data_root.unwrap()), &part.keys)
    };

    Ok(SubgraphResponse {
        data,
        errors: part.errors,
        extensions: None,
        headers: None,
        bytes: part.bytes,
        status: None,
        flat_part: None,
    })
}

fn flat_store_to_value(
    store: &FlatResponseStore<'static>,
    id: FlatValueId,
    keys: &ResponseKeys,
) -> Value<'static> {
    match store.value(id) {
        FlatValue::Null | FlatValue::Missing | FlatValue::Inaccessible => Value::Null,
        FlatValue::Bool(v) => Value::Bool(*v),
        FlatValue::I64(v) => Value::I64(*v),
        FlatValue::U64(v) => Value::U64(*v),
        FlatValue::F64(v) => Value::F64(*v),
        FlatValue::String(s) => Value::String(s.clone()),
        FlatValue::RawJson(raw) => Value::RawJson(raw.clone()),
        FlatValue::Object { fields: _ } | FlatValue::List { items: _ } => {
            // Serialize to JSON and parse back as Value with owned strings
            let mut buf = Vec::new();
            crate::response::flat_store::serialize_store_data(store, keys, id, &mut buf);
            // Use serde_json for owned conversion
            let serde_value: serde_json::Value =
                serde_json::from_slice(&buf).unwrap_or_default();
            serde_value_to_owned_value(serde_value)
        }
    }
}

fn serde_value_to_owned_value(value: serde_json::Value) -> Value<'static> {
    match value {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(b) => Value::Bool(b),
        serde_json::Value::String(s) => Value::String(Cow::Owned(s)),
        serde_json::Value::Number(n) => {
            if let Some(u) = n.as_u64() {
                Value::U64(u)
            } else if let Some(i) = n.as_i64() {
                Value::I64(i)
            } else if let Some(f) = n.as_f64() {
                Value::F64(f)
            } else {
                Value::Null
            }
        }
        serde_json::Value::Array(arr) => {
            let items: Vec<Value<'static>> = arr
                .into_iter()
                .map(serde_value_to_owned_value)
                .collect();
            Value::Array(items)
        }
        serde_json::Value::Object(obj) => {
            let mut entries = Vec::with_capacity(obj.len());
            for (key, val) in obj {
                let owned_val = serde_value_to_owned_value(val);
                let leaked_key: &'static str = Box::leak(key.into_boxed_str());
                entries.push((leaked_key, owned_val));
            }
            Value::Object(entries)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::introspection::schema::FieldNullability;
    use crate::response::flat_plan::{
        FetchTarget, FetchWritePlan, FieldWritePlan, LeafWritePlan, ListWritePlan, ObjectWritePlan,
        ValueWritePlan,
    };

    fn leaf_plan(key: &str, nullable: bool) -> LeafWritePlan {
        LeafWritePlan {
            response_key: key.into(),
            nullability: FieldNullability::leaf(nullable),
            custom_scalar: false,
        }
    }

    fn make_field(
        keys: &mut ResponseKeys,
        source_key: &str,
        value: ValueWritePlan,
        nullable: bool,
    ) -> FieldWritePlan {
        let response_key: Box<str> = source_key.into();
        let response_key_id = keys.intern(&response_key);
        FieldWritePlan {
            source_key: response_key.clone(),
            response_key,
            response_key_id,
            value,
            nullability: FieldNullability::leaf(nullable),
        }
    }

    fn build_fetch_plan(data: ValueWritePlan, keys: ResponseKeys) -> FetchWritePlan {
        FetchWritePlan {
            fetch_id: 0,
            data,
            target: FetchTarget::Root,
            keys: Arc::new(keys),
        }
    }

    #[test]
    fn fused_decode_simple_scalar_object() {
        let mut keys = ResponseKeys::default();

        let plan = ObjectWritePlan {
            response_key: "data".into(),
            nullability: FieldNullability::leaf(true),
            fields: vec![
                make_field(
                    &mut keys,
                    "id",
                    ValueWritePlan::Leaf(leaf_plan("id", true)),
                    true,
                ),
                make_field(
                    &mut keys,
                    "name",
                    ValueWritePlan::Leaf(leaf_plan("name", false)),
                    false,
                ),
            ],
            object_type_name: "Query".into(),
        };

        let fetch_plan = build_fetch_plan(ValueWritePlan::Object(plan), keys);

        let json = br#"{"data":{"id":"1","name":"hello"}}"#;
        let part = deserialize_fetch_into_part(Bytes::from_static(json), &fetch_plan).unwrap();

        assert!(!part.propagated_null);

        let mut buf = Vec::new();
        let root = part.data_root.unwrap();
        crate::response::flat_store::serialize_store_data(&part.store, &part.keys, root, &mut buf);
        let output = std::str::from_utf8(&buf).unwrap();
        assert_eq!(output, "{\"data\":{\"id\":\"1\",\"name\":\"hello\"}}");
    }

    #[test]
    fn fused_decode_skips_unknown_fields() {
        let mut keys = ResponseKeys::default();

        let plan = ObjectWritePlan {
            response_key: "data".into(),
            nullability: FieldNullability::leaf(true),
            fields: vec![make_field(
                &mut keys,
                "id",
                ValueWritePlan::Leaf(leaf_plan("id", true)),
                true,
            )],
            object_type_name: "Query".into(),
        };

        let fetch_plan = build_fetch_plan(ValueWritePlan::Object(plan), keys);

        let json = br#"{"data":{"id":"1","extra":"skip me"}}"#;
        let part = deserialize_fetch_into_part(Bytes::from_static(json), &fetch_plan).unwrap();

        let mut buf = Vec::new();
        crate::response::flat_store::serialize_store_data(
            &part.store,
            &part.keys,
            part.data_root.unwrap(),
            &mut buf,
        );
        assert_eq!(std::str::from_utf8(&buf).unwrap(), "{\"data\":{\"id\":\"1\"}}");
    }

    #[test]
    fn fused_decode_nullable_field_is_null() {
        let mut keys = ResponseKeys::default();

        let plan = ObjectWritePlan {
            response_key: "data".into(),
            nullability: FieldNullability::leaf(true),
            fields: vec![make_field(
                &mut keys,
                "id",
                ValueWritePlan::Leaf(leaf_plan("id", false)),
                false,
            )],
            object_type_name: "Query".into(),
        };

        let fetch_plan = build_fetch_plan(ValueWritePlan::Object(plan), keys);

        let json = br#"{"data":{"id":null}}"#;
        let part = deserialize_fetch_into_part(Bytes::from_static(json), &fetch_plan).unwrap();
        assert!(!part.propagated_null);

        let mut buf = Vec::new();
        crate::response::flat_store::serialize_store_data(
            &part.store,
            &part.keys,
            part.data_root.unwrap(),
            &mut buf,
        );
        assert_eq!(std::str::from_utf8(&buf).unwrap(), "{\"data\":{\"id\":null}}");
    }

    #[test]
    fn fused_decode_nested_object() {
        let mut keys = ResponseKeys::default();

        let inner = ObjectWritePlan {
            response_key: "user".into(),
            nullability: FieldNullability::leaf(false),
            fields: vec![make_field(
                &mut keys,
                "name",
                ValueWritePlan::Leaf(leaf_plan("name", true)),
                true,
            )],
            object_type_name: "User".into(),
        };

        let plan = ObjectWritePlan {
            response_key: "data".into(),
            nullability: FieldNullability::leaf(true),
            fields: vec![make_field(
                &mut keys,
                "user",
                ValueWritePlan::Object(inner),
                false,
            )],
            object_type_name: "Query".into(),
        };

        let fetch_plan = build_fetch_plan(ValueWritePlan::Object(plan), keys);

        let json = br#"{"data":{"user":{"name":"Alice"}}}"#;
        let part = deserialize_fetch_into_part(Bytes::from_static(json), &fetch_plan).unwrap();

        let mut buf = Vec::new();
        crate::response::flat_store::serialize_store_data(
            &part.store,
            &part.keys,
            part.data_root.unwrap(),
            &mut buf,
        );
        assert_eq!(
            std::str::from_utf8(&buf).unwrap(),
            "{\"data\":{\"user\":{\"name\":\"Alice\"}}}"
        );
    }

    #[test]
    fn fused_decode_list_of_objects() {
        let mut keys = ResponseKeys::default();

        let inner = ObjectWritePlan {
            response_key: "item".into(),
            nullability: FieldNullability::leaf(true),
            fields: vec![make_field(
                &mut keys,
                "id",
                ValueWritePlan::Leaf(leaf_plan("id", true)),
                true,
            )],
            object_type_name: "Item".into(),
        };

        let list = ListWritePlan {
            response_key: "items".into(),
            nullability: FieldNullability::leaf(false),
            item: Box::new(ValueWritePlan::Object(inner)),
        };

        let plan = ObjectWritePlan {
            response_key: "data".into(),
            nullability: FieldNullability::leaf(true),
            fields: vec![make_field(
                &mut keys,
                "items",
                ValueWritePlan::List(list),
                false,
            )],
            object_type_name: "Query".into(),
        };

        let fetch_plan = build_fetch_plan(ValueWritePlan::Object(plan), keys);

        let json = br#"{"data":{"items":[{"id":"1"},{"id":"2"}]}}"#;
        let part = deserialize_fetch_into_part(Bytes::from_static(json), &fetch_plan).unwrap();

        let mut buf = Vec::new();
        crate::response::flat_store::serialize_store_data(
            &part.store,
            &part.keys,
            part.data_root.unwrap(),
            &mut buf,
        );
        assert_eq!(
            std::str::from_utf8(&buf).unwrap(),
            "{\"data\":{\"items\":[{\"id\":\"1\"},{\"id\":\"2\"}]}}"
        );
    }

    #[test]
    fn fused_decode_handles_subgraph_errors() {
        let keys = ResponseKeys::default();

        let plan = ObjectWritePlan {
            response_key: "data".into(),
            nullability: FieldNullability::leaf(true),
            fields: vec![],
            object_type_name: "Query".into(),
        };

        let fetch_plan = build_fetch_plan(ValueWritePlan::Object(plan), keys);

        let json = br#"{"data":{},"errors":[{"message":"boo","extensions":{"code":"ERR"}}]}"#;
        let part = deserialize_fetch_into_part(Bytes::from_static(json), &fetch_plan).unwrap();

        let errors = part.errors.unwrap();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].message, "boo");
    }
}
