mod expr_input_val;
mod fetcher;
mod spec;

use hive_router_config::persisted_documents::{BoolOrExpression, PersistedDocumentsConfig};
use hive_router_plan_executor::{
    execution::client_request_details::JwtRequestDetails,
    utils::expression::{compile_expression, execute_expression_with_value},
};
use ntex::web::HttpRequest;
use tracing::trace;
use vrl::{compiler::Program as VrlProgram, core::Value as VrlValue};

use crate::{
    persisted_documents::{
        expr_input_val::get_expression_input_val, fetcher::PersistedDocumentsFetcher,
        spec::PersistedDocumentsSpecResolver,
    },
    pipeline::execution_request::ExecutionRequest,
};

pub struct PersistedDocumentsLoader {
    fetcher: PersistedDocumentsFetcher,
    spec: PersistedDocumentsSpecResolver,
    allow_arbitrary_operations: BoolOrProgram,
}

pub enum BoolOrProgram {
    Bool(bool),
    Program(Box<VrlProgram>),
}

pub fn compile_bool_or_expression(
    bool_or_expr: &BoolOrExpression,
) -> Result<BoolOrProgram, PersistedDocumentsError> {
    match bool_or_expr {
        BoolOrExpression::Bool(b) => Ok(BoolOrProgram::Bool(*b)),
        BoolOrExpression::Expression { expression } => {
            let program = compile_expression(expression, None).map_err(|err| {
                PersistedDocumentsError::ArbitraryOpsExpressionBuild(expression.to_string(), err)
            })?;
            Ok(BoolOrProgram::Program(Box::new(program)))
        }
    }
}

pub fn execute_bool_or_program(
    program: &BoolOrProgram,
    execution_request: &ExecutionRequest,
    req: &HttpRequest,
    jwt_request_details: &JwtRequestDetails<'_>,
) -> Result<bool, PersistedDocumentsError> {
    match program {
        BoolOrProgram::Bool(b) => Ok(*b),
        BoolOrProgram::Program(prog) => {
            let input = get_expression_input_val(execution_request, req, jwt_request_details);
            let output = execute_expression_with_value(prog, input).map_err(|e| {
                PersistedDocumentsError::ArbitraryOpsExpressionExecute(e.to_string())
            })?;

            match output {
                VrlValue::Boolean(b) => Ok(b),
                _ => Err(PersistedDocumentsError::ArbitraryOpsExpressionExecute(
                    format!(
                        "Expected boolean output from allow arbitrary operations expression, got {:?}",
                        output
                    ),
                )),
            }
        }
    }
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
    #[error("Failed to compile VRL expression to extract the document id for the persisted documents '{0}'. Please check your VRL expression for syntax errors. Diagnostic: {1}")]
    SpecExpressionBuild(String, String),
    #[error("Failed to execute VRL expression to extract the document id for the persisted documents: {0}")]
    SpecExpressionExecute(String),
    #[error("Failed to compile VRL expression to decide to allow arbitrary operations '{0}'. Please check your VRL expression for syntax errors. Diagnostic: {1}")]
    ArbitraryOpsExpressionBuild(String, String),
    #[error("Failed to execute VRL expression to decide to allow arbitrary operations: {0}")]
    ArbitraryOpsExpressionExecute(String),
    #[error("Failed to read persisted document: {0}")]
    ReadError(String),
    #[error("Key not found in persisted documents request")]
    KeyNotFound,
}

impl PersistedDocumentsLoader {
    pub fn try_new(config: &PersistedDocumentsConfig) -> Result<Self, PersistedDocumentsError> {
        let fetcher = PersistedDocumentsFetcher::try_new(&config.source)?;

        let spec = PersistedDocumentsSpecResolver::new(&config.spec)?;

        let allow_arbitrary_operations =
            compile_bool_or_expression(&config.allow_arbitrary_operations)?;

        Ok(Self {
            fetcher,
            spec,
            allow_arbitrary_operations,
        })
    }

    pub async fn handle(
        &self,
        execution_request: &mut ExecutionRequest,
        req: &HttpRequest,
        jwt_request_details: &JwtRequestDetails<'_>,
    ) -> Result<(), PersistedDocumentsError> {
        if let Some(ref query) = &execution_request.query {
            if !query.is_empty() {
                trace!("arbitrary operation detected in request");
                let allow_arbitrary_operations = execute_bool_or_program(
                    &self.allow_arbitrary_operations,
                    execution_request,
                    req,
                    jwt_request_details,
                )?;
                // If arbitrary operations are not allowed, return an error.
                if !allow_arbitrary_operations {
                    return Err(PersistedDocumentsError::PersistedDocumentsOnly);
                // If they are allowed, skip fetching persisted document.
                } else {
                    return Ok(());
                }
            }
        }

        trace!("extracting persisted document id from request");
        let document_id =
            self.spec
                .extract_document_id(execution_request, req, jwt_request_details)?;
        trace!("fetching persisted document for id {}", document_id);
        let query = self.fetcher.resolve(&document_id).await?;
        trace!("persisted document fetched successfully {}", query);
        execution_request.query = Some(query);

        Ok(())
    }
}
