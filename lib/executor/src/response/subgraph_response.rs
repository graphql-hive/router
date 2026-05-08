use core::fmt;
use std::sync::Arc;

use bytes::Bytes;
use hive_router_query_planner::planner::plan_nodes::OpaqueScalarPaths;
use http::HeaderMap;
use serde::{
    de::{self, DeserializeSeed, Deserializer, MapAccess, SeqAccess, Visitor},
    Deserialize,
};
use sonic_rs::LazyValue;

use crate::{
    executors::error::SubgraphExecutorError,
    response::{graphql_error::GraphQLError, value::Value},
};

#[derive(Debug, Default)]
pub struct SubgraphResponse<'a> {
    pub data: Value<'a>,
    pub errors: Option<Vec<GraphQLError>>,
    pub extensions: Option<Value<'a>>,
    pub headers: Option<Arc<HeaderMap>>,
    pub bytes: Option<Bytes>,
}

impl<'de> de::Deserialize<'de> for SubgraphResponse<'de> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        SubgraphResponseSeed {
            opaque_scalar_paths: &EMPTY_OPAQUE_SCALAR_PATHS,
        }
        .deserialize(deserializer)
    }
}

static EMPTY_OPAQUE_SCALAR_PATHS: OpaqueScalarPaths = OpaqueScalarPaths {
    children: std::collections::BTreeMap::new(),
    terminal: false,
};

struct SubgraphResponseSeed<'a> {
    opaque_scalar_paths: &'a OpaqueScalarPaths,
}

impl<'a, 'de> DeserializeSeed<'de> for SubgraphResponseSeed<'a> {
    type Value = SubgraphResponse<'de>;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_map(SubgraphResponseVisitor {
            opaque_scalar_paths: self.opaque_scalar_paths,
            _marker: std::marker::PhantomData,
        })
    }
}

struct SubgraphResponseVisitor<'a, 'de> {
    opaque_scalar_paths: &'a OpaqueScalarPaths,
    _marker: std::marker::PhantomData<&'de ()>,
}

impl<'a, 'de> Visitor<'de> for SubgraphResponseVisitor<'a, 'de> {
    type Value = SubgraphResponse<'de>;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a GraphQL response object with data, errors, and extensions fields")
    }

    fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
    where
        M: MapAccess<'de>,
    {
        let mut data = None;
        let mut errors = None;
        let mut extensions = None;

        while let Some(key) = map.next_key::<&str>()? {
            match key {
                "data" => {
                    if data.is_some() {
                        return Err(de::Error::duplicate_field("data"));
                    }
                    data = Some(map.next_value_seed(ValueSeed {
                        opaque_scalar_paths: self.opaque_scalar_paths,
                    })?);
                }
                "errors" => {
                    if errors.is_some() {
                        return Err(de::Error::duplicate_field("errors"));
                    }
                    errors = Some(map.next_value()?);
                }
                "extensions" => {
                    if extensions.is_some() {
                        return Err(de::Error::duplicate_field("extensions"));
                    }
                    // Extensions intentionally stay on the structured path.
                    extensions = Some(map.next_value()?);
                }
                _ => {
                    let _ = map.next_value::<de::IgnoredAny>()?;
                }
            }
        }

        Ok(SubgraphResponse {
            data: data.unwrap_or(Value::Null),
            errors,
            extensions,
            headers: None,
            bytes: None,
        })
    }
}

#[derive(Clone, Copy)]
struct ValueSeed<'a> {
    opaque_scalar_paths: &'a OpaqueScalarPaths,
}

impl<'a, 'de> DeserializeSeed<'de> for ValueSeed<'a> {
    type Value = Value<'de>;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        if self.opaque_scalar_paths.terminal {
            let raw = LazyValue::deserialize(deserializer)?;
            return Ok(Value::RawJson(raw.as_raw_cow()));
        }

        deserializer.deserialize_any(ValueVisitorWithOpaquePaths {
            opaque_scalar_paths: self.opaque_scalar_paths,
        })
    }
}

struct ValueVisitorWithOpaquePaths<'a> {
    opaque_scalar_paths: &'a OpaqueScalarPaths,
}

impl<'a, 'de> Visitor<'de> for ValueVisitorWithOpaquePaths<'a> {
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

    fn visit_borrowed_str<E>(self, value: &'de str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(Value::String(value.into()))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(Value::String(value.to_owned().into()))
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

    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut elements = Vec::with_capacity(seq.size_hint().unwrap_or(0));
        while let Some(elem) = seq.next_element_seed(ValueSeed {
            opaque_scalar_paths: self.opaque_scalar_paths,
        })? {
            elements.push(elem);
        }
        Ok(Value::Array(elements))
    }

    fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
    where
        M: MapAccess<'de>,
    {
        let mut entries = Vec::with_capacity(map.size_hint().unwrap_or(0));
        while let Some(key) = map.next_key::<&'de str>()? {
            let child_paths = self
                .opaque_scalar_paths
                .children
                .get(key)
                .unwrap_or(&EMPTY_OPAQUE_SCALAR_PATHS);
            let value = map.next_value_seed(ValueSeed {
                opaque_scalar_paths: child_paths,
            })?;
            entries.push((key, value));
        }
        entries.sort_unstable_by_key(|(key, _)| *key);
        Ok(Value::Object(entries))
    }
}

impl<'a> SubgraphResponse<'a> {
    pub fn deserialize_from_bytes(
        bytes: Bytes,
        opaque_scalar_paths: Option<&OpaqueScalarPaths>,
    ) -> Result<SubgraphResponse<'static>, SubgraphExecutorError> {
        let bytes_ref: &[u8] = &bytes;

        // SAFETY: The byte slice `bytes_ref` is transmuted to `'static`.
        // This is safe because the returned `SubgraphResponse` stores the `bytes` (Arc-backed
        // reference-counted buffer) in its `bytes` field, keeping the underlying data alive as
        // long as the `SubgraphResponse` does. The `data` field of `SubgraphResponse` contains
        // values that borrow from this buffer, creating a self-referential struct, which is why
        // `unsafe` is required.
        let bytes_ref: &'static [u8] = unsafe { std::mem::transmute(bytes_ref) };
        let mut deserializer = sonic_rs::Deserializer::from_slice(bytes_ref);

        SubgraphResponseSeed {
            opaque_scalar_paths: opaque_scalar_paths.unwrap_or(&EMPTY_OPAQUE_SCALAR_PATHS),
        }
            .deserialize(&mut deserializer)
            .map_err(SubgraphExecutorError::ResponseDeserializationFailure)
            .and_then(|mut resp: SubgraphResponse<'static>| {
                deserializer
                    .end()
                    .map_err(SubgraphExecutorError::ResponseDeserializationFailure)?;
                resp.bytes = Some(bytes);
                Ok(resp)
            })
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use hive_router_query_planner::planner::plan_nodes::OpaqueScalarPaths;

    use crate::response::value::Value;

    #[test]
    fn deserialize_response_without_data_with_errors_with_extensions() {
        let json_response = r#"
        {
            "errors": [
                {
                    "message": "Random error from subgraph",
                    "extensions":{
                        "statusCode": 400
                    }
                }
            ]
        }"#;

        let response: super::SubgraphResponse =
            sonic_rs::from_str(json_response).expect("Failed to deserialize");

        assert!(response.data.is_null());
        let errors = response.errors.as_ref().unwrap();
        insta::assert_snapshot!(sonic_rs::to_string_pretty(&errors).unwrap(), @r###"
        [
          {
            "message": "Random error from subgraph",
            "extensions": {
              "statusCode": 400
            }
          }
        ]"###);
    }

    #[test]
    fn deserializes_opaque_scalar_data_field_as_raw_json() {
        let mut paths = OpaqueScalarPaths::default();
        paths.insert_path(["labels"]);

        let response = super::SubgraphResponse::deserialize_from_bytes(
            Bytes::from_static(br#"{"data":{"labels":{"generic.learnMore.button\t":"Learn more"}},"extensions":{"statusCode":200}}"#),
            Some(&paths),
        )
        .unwrap();

        let data = response.data.as_object().unwrap();
        assert!(matches!(data[0].1, Value::RawJson(_)));

        let extensions = response.extensions.unwrap();
        assert!(matches!(extensions, Value::Object(_)));
    }
}
