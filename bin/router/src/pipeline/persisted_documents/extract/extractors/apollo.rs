use super::super::super::types::PersistedDocumentId;
use super::super::core::{DocumentIdSourceExtractor, ExtractionContext};

pub(crate) const APOLLO_HASH_PATH: &str = "extensions.persistedQuery.sha256Hash";
pub(crate) const APOLLO_HASH_PATH_SEGMENTS: &[&str; 3] =
    &["extensions", "persistedQuery", "sha256Hash"];

/// Extracts "$.extensions.persistedQuery.sha256Hash" from the GraphQL request body.
pub(crate) struct ApolloExtractor;

impl DocumentIdSourceExtractor for ApolloExtractor {
    fn extract(&self, ctx: &ExtractionContext<'_>) -> Option<PersistedDocumentId> {
        ctx.json_path(APOLLO_HASH_PATH_SEGMENTS)
            .and_then(|value| PersistedDocumentId::try_from(value.as_ref()).ok())
    }
}
