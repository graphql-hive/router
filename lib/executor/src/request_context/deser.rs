use serde::ser::SerializeMap;
use serde::{Serialize, Serializer};
use sonic_rs::{JsonContainerTrait, JsonValueTrait, Value};

use super::{RequestContext, SelectedRequestContext};
use crate::request_context::{RequestContextDomain, RequestContextError};

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
        let mut len = self.custom.size();
        len += self.operation.serialized_len();
        len += self.progressive_override.serialized_len();
        len += self.authentication.serialized_len();
        len += self.telemetry.serialized_len();

        let mut map = serializer.serialize_map(Some(len))?;
        self.operation.serialize_all(&mut map)?;
        self.progressive_override.serialize_all(&mut map)?;
        self.authentication.serialize_all(&mut map)?;
        self.telemetry.serialize_all(&mut map)?;
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
            if self.context.operation.is_applicable(key) {
                self.context.operation.serialize_entry(key, &mut map)?;
                continue;
            }

            if self.context.progressive_override.is_applicable(key) {
                self.context
                    .progressive_override
                    .serialize_entry(key, &mut map)?;
                continue;
            }

            if self.context.authentication.is_applicable(key) {
                self.context.authentication.serialize_entry(key, &mut map)?;
                continue;
            }

            if self.context.telemetry.is_applicable(key) {
                self.context.telemetry.serialize_entry(key, &mut map)?;
                continue;
            }

            if let Some(value) = self.context.custom.get(key) {
                map.serialize_entry(key, value)?;
            }
        }

        map.end()
    }
}
