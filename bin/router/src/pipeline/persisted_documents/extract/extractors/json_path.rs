use std::borrow::Cow;
use std::collections::HashMap;

use sonic_rs::{JsonValueTrait, OwnedLazyValue, Value};

use super::super::super::types::PersistedDocumentId;
use super::super::core::{DocumentIdSourceExtractor, ExtractionContext};

/// Extracts "$.x.y.z" from the GraphQL request body.
pub(crate) struct JsonPathExtractor {
    // TODO: Add e2e coverage for JSON-path extraction edge cases
    pub(crate) segments: Vec<String>,
}

impl DocumentIdSourceExtractor for JsonPathExtractor {
    fn extract(&self, ctx: &ExtractionContext<'_>) -> Option<PersistedDocumentId> {
        ctx.json_path(&self.segments)
            .and_then(|value| PersistedDocumentId::try_from(value.as_ref()).ok())
    }
}

impl JsonPathExtractor {
    pub(crate) fn requires_nonstandard_json_fields(segments: &[String]) -> bool {
        // We only skip capturing unknown top-level fields when extraction can be
        // satisfied by extensions.*
        // Everything else requires captured nonstandard JSON fields.
        if segments.is_empty() {
            // Config validation rejects empty json_path
            return false;
        }

        segments[0] != "extensions"
    }
}

impl<'a> ExtractionContext<'a> {
    pub(super) fn json_path<T: AsRef<str>>(&self, segments: &[T]) -> Option<Cow<'a, str>> {
        let (first, rest) = segments.split_first()?;
        let first = first.as_ref();

        match first {
            // We don't support JSON paths for these fields
            "query" | "operationName" | "variables" => None,
            "extensions" => self
                .graphql_params
                .extensions
                .as_ref()
                .and_then(|obj| Self::extract_from_object_map(obj, rest)),
            other => self
                .nonstandard_json_fields
                .and_then(|map| map.get(other))
                .and_then(|value| extract_from_json_path(value, rest)),
        }
    }

    fn extract_from_object_map<'b, T: AsRef<str>>(
        obj: &'b HashMap<String, Value>,
        segments: &[T],
    ) -> Option<Cow<'b, str>> {
        let (first, rest) = segments.split_first()?;
        let value = obj.get(first.as_ref())?;
        extract_from_json_path(value, rest)
    }
}

/// The trait exist to reuse `extract_from_json_path` logic across different types
pub(crate) trait JsonPathNode {
    fn get_child(&self, key: &str) -> Option<&Self>;
    fn as_document_id_value(&self) -> Option<Cow<'_, str>>;
}

/// It's for extraction of document id from `extensions.*`
impl JsonPathNode for Value {
    #[inline]
    fn get_child(&self, key: &str) -> Option<&Self> {
        self.get(key)
    }

    #[inline]
    fn as_document_id_value(&self) -> Option<Cow<'_, str>> {
        if let Some(value) = self.as_str() {
            return Some(Cow::Borrowed(value));
        }

        self.as_u64().map(|value| Cow::Owned(value.to_string()))
    }
}

/// It's for extraction of document id from non-standard fields
impl JsonPathNode for OwnedLazyValue {
    #[inline]
    fn get_child(&self, key: &str) -> Option<&Self> {
        self.get(key)
    }

    #[inline]
    fn as_document_id_value(&self) -> Option<Cow<'_, str>> {
        if let Some(value) = self.as_str() {
            return Some(Cow::Borrowed(value));
        }

        self.as_u64().map(|value| Cow::Owned(value.to_string()))
    }
}

#[inline]
pub(crate) fn extract_from_json_path<'a, N, T>(value: &'a N, segments: &[T]) -> Option<Cow<'a, str>>
where
    N: JsonPathNode,
    T: AsRef<str>,
{
    let mut current = value;
    for segment in segments {
        current = current.get_child(segment.as_ref())?;
    }

    current.as_document_id_value()
}
