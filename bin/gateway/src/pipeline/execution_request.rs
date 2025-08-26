use std::collections::HashMap;

use axum::body::Body;
use axum::extract::Query;
use http::{Method, Request};
use http_body_util::BodyExt;
use serde::Deserialize;
use sonic_rs::Value;
use tracing::{trace, warn};

use crate::pipeline::error::{PipelineError, PipelineErrorFromAcceptHeader, PipelineErrorVariant};
use crate::pipeline::header::AssertRequestJson;

#[derive(serde::Deserialize, Debug)]
struct GETQueryParams {
    pub query: Option<String>,
    #[serde(rename = "camelCase")]
    pub operation_name: Option<String>,
    pub variables: Option<String>,
    pub extensions: Option<String>,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionRequest {
    pub query: String,
    pub operation_name: Option<String>,
    pub variables: Option<HashMap<String, Value>>,
    // TODO: We don't use extensions yet, but we definitely will in the future.
    #[allow(dead_code)]
    pub extensions: Option<HashMap<String, Value>>,
}

impl TryInto<ExecutionRequest> for GETQueryParams {
    type Error = PipelineErrorVariant;

    fn try_into(self) -> Result<ExecutionRequest, Self::Error> {
        let query = match self.query {
            Some(q) => q,
            None => return Err(PipelineErrorVariant::GetMissingQueryParam("query")),
        };

        let variables = match self.variables.as_deref() {
            Some(v_str) if !v_str.is_empty() => match sonic_rs::from_str(v_str) {
                Ok(vars) => Some(vars),
                Err(e) => {
                    return Err(PipelineErrorVariant::FailedToParseVariables(e));
                }
            },
            _ => None,
        };

        let extensions = match self.extensions.as_deref() {
            Some(e_str) if !e_str.is_empty() => match sonic_rs::from_str(e_str) {
                Ok(exts) => Some(exts),
                Err(e) => {
                    return Err(PipelineErrorVariant::FailedToParseExtensions(e));
                }
            },
            _ => None,
        };

        let execution_request = ExecutionRequest {
            query,
            operation_name: self.operation_name,
            variables,
            extensions,
        };

        Ok(execution_request)
    }
}

#[inline]
pub async fn get_execution_request(
    req: &mut Request<Body>,
) -> Result<ExecutionRequest, PipelineError> {
    let http_method = req.method();
    let execution_request: ExecutionRequest = match *http_method {
        Method::GET => {
            trace!("processing GET GraphQL operation");

            let query_params = Query::<GETQueryParams>::try_from_uri(req.uri())
                .map_err(|qe| {
                    req.new_pipeline_error(PipelineErrorVariant::GetInvalidQueryParams(qe))
                })?
                .0;

            trace!("parsed GET query params: {:?}", query_params);

            query_params
                .try_into()
                .map_err(|err| req.new_pipeline_error(err))?
        }
        Method::POST => {
            trace!("Processing POST GraphQL request");

            req.assert_json_content_type()?;

            let body_bytes = req
                .body_mut()
                .collect()
                .await
                .map_err(|err| {
                    warn!("Failed to read body bytes: {}", err);
                    req.new_pipeline_error(PipelineErrorVariant::FailedToReadBodyBytes(err))
                })?
                .to_bytes();

            let execution_request = unsafe {
                sonic_rs::from_slice_unchecked::<ExecutionRequest>(&body_bytes).map_err(|e| {
                    warn!("Failed to parse body: {}", e);
                    req.new_pipeline_error(PipelineErrorVariant::FailedToParseBody(e))
                })?
            };

            execution_request
        }
        _ => {
            warn!("unsupported HTTP method: {}", http_method);

            return Err(
                req.new_pipeline_error(PipelineErrorVariant::UnsupportedHttpMethod(
                    http_method.to_owned(),
                )),
            );
        }
    };

    Ok(execution_request)
}
