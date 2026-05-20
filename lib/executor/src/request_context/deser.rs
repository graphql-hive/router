use serde::ser::SerializeMap;
use serde::{Serialize, Serializer};
use sonic_rs::{JsonContainerTrait, JsonValueTrait, Value};

use super::error::RequestContextError;
use super::{RequestContext, SelectedRequestContext};

pub trait RequestContextValueExt {
    fn expect_str<'a>(
        &'a self,
        key: &'static str,
        expected: &'static str,
    ) -> Result<&'a str, RequestContextError>;
    fn expect_array<'a>(
        &'a self,
        key: &'static str,
        expected: &'static str,
    ) -> Result<&'a [Value], RequestContextError>;
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

    fn expect_array<'a>(
        &'a self,
        key: &'static str,
        allowed_types: &'static str,
    ) -> Result<&'a [Value], RequestContextError> {
        let value =
            self.as_array()
                .ok_or_else(|| RequestContextError::ReservedKeyTypeMismatch {
                    key: key.to_string(),
                    expected: allowed_types,
                })?;

        Ok(value.as_slice())
    }
}

impl Serialize for RequestContext {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let len = self.custom.size() + self.reserved_serialized_len();

        let mut map = serializer.serialize_map(Some(len))?;
        self.serialize_all_reserved(&mut map)?;
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

        let keys = self.selection.keys();
        let mut map = serializer.serialize_map(Some(keys.len()))?;

        for key in keys {
            if self.context.try_serialize_reserved_entry(key, &mut map)? {
                continue;
            }

            if let Some(value) = self.context.custom.get(key) {
                map.serialize_entry(key, value)?;
            }
        }

        map.end()
    }
}
