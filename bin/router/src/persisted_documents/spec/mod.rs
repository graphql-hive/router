use hive_router_config::persisted_documents::PersistedDocumentsSpec;
use hive_router_plan_executor::{
    execution::client_request_details::JwtRequestDetails,
    utils::expression::{compile_expression, execute_expression_with_value},
};
use ntex::web::HttpRequest;
use sonic_rs::JsonValueTrait;
use vrl::{compiler::Program as VrlProgram, core::Value as VrlValue};

use crate::{
    persisted_documents::{expr_input_val::get_expression_input_val, PersistedDocumentsError},
    pipeline::execution_request::ExecutionRequest,
};

pub enum PersistedDocumentsSpecResolver {
    Hive,
    Apollo,
    Relay,
    Expression(Box<VrlProgram>),
}

impl PersistedDocumentsSpecResolver {
    pub fn new(spec: &PersistedDocumentsSpec) -> Result<Self, PersistedDocumentsError> {
        match spec {
            PersistedDocumentsSpec::Hive => Ok(PersistedDocumentsSpecResolver::Hive),
            PersistedDocumentsSpec::Apollo => Ok(PersistedDocumentsSpecResolver::Apollo),
            PersistedDocumentsSpec::Relay => Ok(PersistedDocumentsSpecResolver::Relay),
            PersistedDocumentsSpec::Expression(expression) => {
                let program = compile_expression(expression, None).map_err(|err| {
                    PersistedDocumentsError::SpecExpressionBuild(expression.to_string(), err)
                })?;
                Ok(PersistedDocumentsSpecResolver::Expression(Box::new(
                    program,
                )))
            }
        }
    }
    pub fn extract_document_id(
        &self,
        execution_request: &ExecutionRequest,
        req: &HttpRequest,
        jwt_request_details: &JwtRequestDetails<'_>,
    ) -> Result<String, PersistedDocumentsError> {
        match &self {
            PersistedDocumentsSpecResolver::Hive => execution_request
                .extra_params
                .get("documentId")
                .and_then(|val| val.as_str().map(|s| s.to_string()))
                .ok_or(PersistedDocumentsError::KeyNotFound),
            PersistedDocumentsSpecResolver::Apollo => execution_request
                .extensions
                .get("persistedQuery")
                .and_then(|val| val.get("sha256Hash"))
                .and_then(|val| val.as_str().map(|s| s.to_string()))
                .ok_or(PersistedDocumentsError::KeyNotFound),
            PersistedDocumentsSpecResolver::Relay => execution_request
                .extra_params
                .get("doc_id")
                .and_then(|s| s.as_str().map(|s| s.to_string()))
                .ok_or(PersistedDocumentsError::KeyNotFound),
            PersistedDocumentsSpecResolver::Expression(program) => {
                let input = get_expression_input_val(execution_request, req, jwt_request_details);

                let output = execute_expression_with_value(program, input)
                    .map_err(|e| PersistedDocumentsError::SpecExpressionExecute(e.to_string()))?;

                match output {
                    VrlValue::Bytes(b) => Ok(String::from_utf8_lossy(&b).to_string()),
                    _ => Err(PersistedDocumentsError::SpecExpressionExecute(format!(
                        "Expected string output from persisted documents expression, got {:?}",
                        output
                    ))),
                }
            }
        }
    }
}
