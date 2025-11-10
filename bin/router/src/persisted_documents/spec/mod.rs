use std::collections::BTreeMap;

use hive_router_config::persisted_documents::PersistedDocumentsSpec;
use hive_router_plan_executor::{execution::client_request_details::{JwtRequestDetails, client_header_map_to_vrl_value, client_url_to_vrl_value}, utils::expression::{compile_expression, execute_expression_with_value}};
use ntex::web::HttpRequest;
use sonic_rs::JsonValueTrait;
use vrl::{compiler::Program as VrlProgram, core::Value as VrlValue};

use crate::{persisted_documents::PersistedDocumentsError, pipeline::execution_request::ExecutionRequest};

pub enum PersistedDocumentsSpecResolver {
    Hive,
    Apollo,
    Relay,
    Expression(VrlProgram),
}

impl PersistedDocumentsSpecResolver {
    pub fn new(spec: &PersistedDocumentsSpec) -> Result<Self, PersistedDocumentsError> {
        match spec {
            PersistedDocumentsSpec::Hive => Ok(PersistedDocumentsSpecResolver::Hive),
            PersistedDocumentsSpec::Apollo => Ok(PersistedDocumentsSpecResolver::Apollo),
            PersistedDocumentsSpec::Relay => Ok(PersistedDocumentsSpecResolver::Relay),
            PersistedDocumentsSpec::Expression(expr) => {
                let program = compile_expression(expr, None)
                    .map_err(|err| PersistedDocumentsError::ExpressionBuild(expr.to_string(), err))?;
                Ok(PersistedDocumentsSpecResolver::Expression(program))
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
            PersistedDocumentsSpecResolver::Hive => {
                match execution_request.extra_params.get("documentId") {
                    Some(val) => val.as_str()
                                    .map(|s| s.to_string())
                                    .ok_or(PersistedDocumentsError::KeyNotFound),
                    None => Err(PersistedDocumentsError::KeyNotFound)
                }
            },
            PersistedDocumentsSpecResolver::Apollo => {
                match execution_request.extra_params.get("extensions") {
                    Some(extensions) => {
                        match extensions.get("persistedQuery") {
                            Some(persisted_query) => {
                                match persisted_query.get("sha256Hash") {
                                    Some(hash) => 
                                        hash.as_str()
                                            .map(|s| s.to_string())
                                            .ok_or(PersistedDocumentsError::KeyNotFound),
                                    None => Err(PersistedDocumentsError::KeyNotFound),
                                }
                            },
                                None => Err(PersistedDocumentsError::KeyNotFound)
                        }
                    },
                    None => Err(PersistedDocumentsError::KeyNotFound)
                }
            },
            PersistedDocumentsSpecResolver::Relay => {
                match execution_request.extra_params.get("doc_id") {
                    Some(val) => val.as_str()
                                    .map(|s| s.to_string())
                                    .ok_or(PersistedDocumentsError::KeyNotFound),
                    None => Err(PersistedDocumentsError::KeyNotFound)
                }
            },
            PersistedDocumentsSpecResolver::Expression(program) => {
                let headers_value = client_header_map_to_vrl_value(req.headers());
                let url_value = client_url_to_vrl_value(req.uri());
                let request_obj = VrlValue::Object(
                    BTreeMap::from([
                    ("method".into(), req.method().as_str().into()),
                    ("headers".into(), headers_value),
                    ("url".into(), url_value),
                    ("jwt".into(), jwt_request_details.into()),
                ]));
                let input = VrlValue::Object(BTreeMap::from([
                    ("request".into(), request_obj),
                ]));

                let output = execute_expression_with_value(program, input)
                    .map_err(|e| PersistedDocumentsError::ExpressionExecute(e.to_string()))?;

                match output {
                    VrlValue::Bytes(b) => Ok(String::from_utf8_lossy(&b).to_string()),
                    _ => Err(PersistedDocumentsError::ExpressionExecute(
                        format!("Expected string output from persisted documents expression, got {:?}", output)
                    )),
                }
            }
        }
    }
}