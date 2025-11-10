mod fetcher;
mod spec;


use hive_router_config::
    persisted_documents::{PersistedDocumentsConfig}
;
use hive_router_plan_executor::execution::client_request_details::JwtRequestDetails;
use ntex::web::HttpRequest;

use crate::{persisted_documents::{fetcher::PersistedDocumentsFetcher, spec::PersistedDocumentsSpecResolver}, pipeline::execution_request::ExecutionRequest};

pub struct PersistedDocumentsLoader {
    fetcher: PersistedDocumentsFetcher,
    spec: PersistedDocumentsSpecResolver,
    allow_arbitrary_operations: bool,
}


#[derive(Debug, thiserror::Error)]
pub enum PersistedDocumentsError {
    #[error("Persisted document not found: {0}")]
    NotFound(String),
    #[error("Only persisted documents are allowed")]
    PersistedDocumentsOnly,
    #[error("Network error: {0}")]
    NetworkError(reqwest_middleware::Error),
    #[error("Failed to read persisted documents from file: {0}")]
    FileReadError(std::io::Error),
    #[error("Failed to parse persisted documents: {0}")]
    ParseError(serde_json::Error),
    #[error("Failed to compile VRL expression for the persisted documents '{0}'. Please check your VRL expression for syntax errors. Diagnostic: {1}")]
    ExpressionBuild(String, String),
    #[error("Failed to execute VRL expression for the persisted documents: {0}")]
    ExpressionExecute(String),
    #[error("Failed to read persisted document: {0}")]
    ReadError(String),
    #[error("Key not found in persisted documents request")]
    KeyNotFound,
}

impl PersistedDocumentsLoader {
    pub fn try_new(
        config: &PersistedDocumentsConfig,
    ) -> Result<Self, PersistedDocumentsError> {
        let fetcher = PersistedDocumentsFetcher::try_new(&config.source)?;

        let spec = PersistedDocumentsSpecResolver::new(&config.spec)?;

        Ok(Self {
            fetcher,
            spec,
            allow_arbitrary_operations: config.allow_arbitrary_operations,
        })
    }

    pub async fn handle(
        &self,
        execution_request: &mut ExecutionRequest,
        req: &HttpRequest,
        jwt_request_details: &JwtRequestDetails<'_>,
    ) -> Result<(), PersistedDocumentsError> {
        if let Some(ref query) = &execution_request.query {
            if (!self.allow_arbitrary_operations) && !query.is_empty() {
                return Err(PersistedDocumentsError::PersistedDocumentsOnly);
            }
            return Ok(());
        }

        let document_id = self.spec.extract_document_id(execution_request, req, jwt_request_details)?;

        let query = self.fetcher.resolve(&document_id).await?;
        execution_request.query = Some(query);

        Ok(())
    }
}

