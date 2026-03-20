use super::super::super::types::PersistedDocumentId;
use super::super::core::{DocumentIdSourceExtractor, ExtractionContext};

pub(crate) const DOCUMENT_ID_FIELD: &str = "documentId";

/// Extracts "$.documentId" from the GraphQL request body.
pub(crate) struct DocumentIdExtractor;

impl DocumentIdSourceExtractor for DocumentIdExtractor {
    fn extract(&self, ctx: &ExtractionContext<'_>) -> Option<PersistedDocumentId> {
        PersistedDocumentId::from_option(ctx.document_id())
    }
}
