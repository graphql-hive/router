use hive_console_sdk::persisted_documents::PersistedDocumentsManager;
use hive_router_config::persisted_documents::PersistedDocumentsSource;

use crate::persisted_documents::{
    fetcher::file::FilePersistedDocumentsManager, PersistedDocumentsError,
};

mod file;
pub enum PersistedDocumentsFetcher {
    File(FilePersistedDocumentsManager),
    HiveConsole(PersistedDocumentsManager),
}

impl PersistedDocumentsFetcher {
    pub fn try_new(config: &PersistedDocumentsSource) -> Result<Self, PersistedDocumentsError> {
        match config {
            PersistedDocumentsSource::File { path, .. } => {
                let manager = FilePersistedDocumentsManager::try_new(path)?;
                Ok(PersistedDocumentsFetcher::File(manager))
            }
            PersistedDocumentsSource::HiveConsole {
                endpoint,
                key,
                accept_invalid_certs,
                request_timeout,
                connect_timeout,
                retry_count,
                cache_size,
            } => {
                let manager = PersistedDocumentsManager::new(
                    key.clone(),
                    endpoint.clone(),
                    *accept_invalid_certs,
                    *connect_timeout,
                    *request_timeout,
                    *retry_count,
                    *cache_size,
                );

                Ok(PersistedDocumentsFetcher::HiveConsole(manager))
            }
        }
    }
    pub async fn resolve(&self, document_id: &str) -> Result<String, PersistedDocumentsError> {
        match self {
            PersistedDocumentsFetcher::File(manager) => Ok(manager.resolve_document(document_id)?),
            PersistedDocumentsFetcher::HiveConsole(manager) => {
                Ok(manager.resolve_document(document_id).await?)
            }
        }
    }
}

impl From<hive_console_sdk::persisted_documents::PersistedDocumentsError>
    for PersistedDocumentsError
{
    fn from(
        orig_err: hive_console_sdk::persisted_documents::PersistedDocumentsError,
    ) -> PersistedDocumentsError {
        match orig_err {
            hive_console_sdk::persisted_documents::PersistedDocumentsError::DocumentNotFound => PersistedDocumentsError::NotFound("unknown".to_string()),
            hive_console_sdk::persisted_documents::PersistedDocumentsError::FailedToFetchFromCDN(e) => PersistedDocumentsError::NetworkError(e),
            hive_console_sdk::persisted_documents::PersistedDocumentsError::PersistedDocumentRequired => PersistedDocumentsError::PersistedDocumentsOnly,
            hive_console_sdk::persisted_documents::PersistedDocumentsError::FailedToParseBody(e) => PersistedDocumentsError::ParseError(e),
            hive_console_sdk::persisted_documents::PersistedDocumentsError::KeyNotFound => PersistedDocumentsError::KeyNotFound,
            hive_console_sdk::persisted_documents::PersistedDocumentsError::FailedToReadCDNResponse(e) => PersistedDocumentsError::NetworkError(
                reqwest_middleware::Error::Reqwest(e),
            ),
            hive_console_sdk::persisted_documents::PersistedDocumentsError::FailedToReadBody(e) => PersistedDocumentsError::ReadError(e),
        }
    }
}
