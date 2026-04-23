use super::super::domains::{RequestContext, RequestContextDomain, HIVE_PREFIX};
use super::super::error::RequestContextError;

use crate::response::value::Value as ResponseValue;
use serde::de::{MapAccess, Visitor};
use serde::{Deserialize, Deserializer};
use std::fmt;

impl RequestContext {
    pub fn for_coprocessor(&mut self) -> RequestContextCoprocessorApi<'_> {
        RequestContextCoprocessorApi::new(self)
    }
}

pub struct RequestContextCoprocessorApi<'a> {
    context: &'a mut RequestContext,
}

impl RequestContextCoprocessorApi<'_> {
    pub fn new(context: &mut RequestContext) -> RequestContextCoprocessorApi<'_> {
        RequestContextCoprocessorApi { context }
    }

    fn set(&mut self, key: &str, value: ResponseValue<'_>) -> Result<(), RequestContextError> {
        if !key.starts_with(HIVE_PREFIX) {
            self.context.custom.apply(key, value);
            return Ok(());
        }

        if self.context.operation.is_applicable(key) {
            self.context
                .operation
                .set_key_value(key, value.as_ref().into())?;
            return Ok(());
        }

        if self.context.progressive_override.is_applicable(key) {
            self.context
                .progressive_override
                .set_key_value(key, value.as_ref().into())?;
            return Ok(());
        }

        if self.context.authentication.is_applicable(key) {
            self.context
                .authentication
                .set_key_value(key, value.as_ref().into())?;
            return Ok(());
        }

        Err(RequestContextError::UnknownReservedKey {
            key: key.to_string(),
        })
    }

    pub fn apply_patch(&mut self, patch: RequestContextPatch) -> Result<(), RequestContextError> {
        for (key, value) in patch.entries {
            self.set(key, value)?;
        }

        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct RequestContextPatch<'a> {
    pub(crate) entries: Vec<(&'a str, ResponseValue<'a>)>,
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
