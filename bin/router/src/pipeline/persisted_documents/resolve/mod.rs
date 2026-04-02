use async_trait::async_trait;
use std::sync::Arc;

use crate::pipeline::error::PipelineError;
use crate::pipeline::persisted_documents::types::{ClientIdentity, PersistedDocumentId};
use file::FileResolverError;
use hive::HiveResolverError;

pub mod file;
pub mod hive;

pub use file::{FileManifestReloadTask, FileManifestResolver};
pub use hive::HiveCDNResolver;

#[derive(Debug, Clone, Copy)]
pub struct PersistedDocumentResolveInput<'a> {
    pub persisted_document_id: &'a PersistedDocumentId,
    pub client_identity: ClientIdentity<'a>,
}

#[derive(Debug, thiserror::Error)]
pub enum PersistedDocumentResolverError {
    #[error("Persisted document not found: {0}")]
    NotFound(String),
    #[error("Persisted documents configuration error: {0}")]
    Configuration(String),
    #[error("Persisted documents storage is not configured")]
    StorageNotConfigured,
    #[error("Hive Storage: {0}")]
    Hive(#[from] HiveResolverError),
    #[error("File Storage: {0}")]
    File(#[from] FileResolverError),
}

impl From<PersistedDocumentResolverError> for PipelineError {
    fn from(value: PersistedDocumentResolverError) -> Self {
        match value {
            PersistedDocumentResolverError::NotFound(document_id) => {
                PipelineError::PersistedDocumentNotFound(document_id)
            }
            PersistedDocumentResolverError::Hive(HiveResolverError::InvalidDocumentIdFormat(_))
            | PersistedDocumentResolverError::Hive(HiveResolverError::ClientIdentityMissing)
            | PersistedDocumentResolverError::Hive(HiveResolverError::ClientIdentityPartial) => {
                PipelineError::PersistedDocumentExtraction(value.to_string())
            }
            other => PipelineError::PersistedDocumentResolution(other.to_string()),
        }
    }
}

#[derive(Debug)]
pub struct ResolvedDocument {
    pub text: Arc<str>,
}

#[async_trait]
pub trait PersistedDocumentResolver: Send + Sync {
    async fn resolve(
        &self,
        input: PersistedDocumentResolveInput<'_>,
    ) -> Result<ResolvedDocument, PersistedDocumentResolverError>;
}
