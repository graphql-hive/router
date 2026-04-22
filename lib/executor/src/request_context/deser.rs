use crate::request_context::operation::OperationContext;
use crate::response::value::Value as ResponseValue;
use serde::de::{MapAccess, Visitor};
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sonic_rs::{JsonValueTrait, Value};
use std::fmt;

use super::{RequestContext, RequestContextPatch, SelectedRequestContext};
use crate::request_context::{RequestContextDomain, RequestContextError};

pub trait RequestContextValueExt {
    fn expect_str<'a>(
        &'a self,
        key: &'static str,
        expected: &'static str,
    ) -> Result<&'a str, RequestContextError>;
}

impl RequestContextValueExt for Value {
    fn expect_str<'a>(
        &'a self,
        key: &'static str,
        allowed_types: &'static str,
    ) -> Result<&'a str, RequestContextError> {
        let value = self
            .as_str()
            .ok_or_else(|| RequestContextError::ReservedKeyTypeMismatch {
                key: key.to_string(),
                expected: allowed_types,
            })?;

        Ok(value)
    }
}

impl Serialize for RequestContext {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut len = self.custom.size();
        len += usize::from(self.operation.name.is_some());
        len += usize::from(self.operation.kind.is_some());

        let mut map = serializer.serialize_map(Some(len))?;
        self.operation.serialize_all(&mut map)?;
        for (key, value) in self.custom.iter() {
            map.serialize_entry(key, value)?;
        }

        map.end()
    }
}

impl Serialize for SelectedRequestContext<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if self.selection.is_all() {
            return self.context.serialize(serializer);
        }

        let keys = self.selection.keys_slice();
        let mut map = serializer.serialize_map(Some(keys.len()))?;

        for key in keys {
            if key.starts_with(OperationContext::DOMAIN_PREFIX) {
                self.context.operation.serialize_entry(key, &mut map)?;
                continue;
            }

            if let Some(value) = self.context.custom.get(key) {
                map.serialize_entry(key, value)?;
            }
        }

        map.end()
    }
}

impl<'a, 'de: 'a> Deserialize<'de> for RequestContextPatch<'a> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct RequestContextPatchVisitor;

        impl<'de> Visitor<'de> for RequestContextPatchVisitor {
            type Value = RequestContextPatch<'de>;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a flat request-context patch object")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut patch = RequestContextPatch::default();
                while let Some((key, value)) = map.next_entry::<&'de str, ResponseValue<'de>>()? {
                    patch.entries.push((key, value));
                }

                Ok(patch)
            }
        }

        deserializer.deserialize_map(RequestContextPatchVisitor)
    }
}
