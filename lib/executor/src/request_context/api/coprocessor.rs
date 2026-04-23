use super::super::domains::{RequestContext, HIVE_PREFIX};
use super::super::error::RequestContextError;

use crate::response::value::Value as ResponseValue;
use serde::de::{MapAccess, Visitor};
use serde::{Deserialize, Deserializer};
use std::fmt;

impl RequestContext {
    /// Returns a coprocessor-facing API handle.
    /// This API supports untyped key-value patching from external JSON payloads.
    pub fn for_coprocessor(&mut self) -> RequestContextCoprocessorApi<'_> {
        RequestContextCoprocessorApi::new(self)
    }
}

/// An API for external coprocessors to mutate the request context.
/// Unlike the Plugin API, this uses string keys and dynamic JSON values.
/// It performs runtime validation and prefix-routing for reserved keys.
pub struct RequestContextCoprocessorApi<'a> {
    context: &'a mut RequestContext,
}

impl RequestContextCoprocessorApi<'_> {
    pub fn new(context: &mut RequestContext) -> RequestContextCoprocessorApi<'_> {
        RequestContextCoprocessorApi { context }
    }

    /// Sets a value for a specific key in the context.
    /// If the key starts with `hive::`, it is routed to a reserved domain.
    /// Otherwise, it is stored in the custom context.
    fn set(&mut self, key: &str, value: ResponseValue<'_>) -> Result<(), RequestContextError> {
        if !key.starts_with(HIVE_PREFIX) {
            self.context.custom.apply(key, value);
            return Ok(());
        }

        self.context
            .try_set_reserved_key(key, value.as_ref().into())
    }

    /// Applies multiple context updates from an external patch object.
    pub fn apply_patch(&mut self, patch: RequestContextPatch) -> Result<(), RequestContextError> {
        for (key, value) in patch.entries {
            self.set(key, value)?;
        }

        Ok(())
    }
}

/// A collection of context updates received from an external coprocessor.
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
