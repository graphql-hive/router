use std::collections::HashMap;

use http::Method;
use ntex::util::Bytes;
use ntex::web::types::Query;
use ntex::web::HttpRequest;
use serde::{Deserialize, Deserializer};
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
    #[serde(default, deserialize_with = "deserialize_null_default")]
    pub variables: HashMap<String, Value>,
    // TODO: We don't use extensions yet, but we definitely will in the future.
    #[allow(dead_code)]
    pub extensions: Option<HashMap<String, Value>>,
}

fn deserialize_null_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    T: Default + Deserialize<'de>,
    D: Deserializer<'de>,
{
    let opt = Option::<T>::deserialize(deserializer)?;
    Ok(opt.unwrap_or_default())
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
                Ok(vars) => vars,
                Err(e) => {
                    return Err(PipelineErrorVariant::FailedToParseVariables(e));
                }
            },
            _ => HashMap::new(),
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
pub async fn get_execution_request_from_http_request(
    req: &mut HttpRequest,
    body_bytes: Bytes,
) -> Result<ExecutionRequest, PipelineError> {
    let http_method = req.method();
    let execution_request: ExecutionRequest = match *http_method {
        Method::GET => {
            trace!("Processing GET GraphQL operation");
            let query_params_str = req.uri().query().ok_or_else(|| {
                req.new_pipeline_error(PipelineErrorVariant::GetInvalidQueryParams)
            })?;
            let query_params = Query::<GETQueryParams>::from_query(query_params_str)
                .map_err(|e| {
                    req.new_pipeline_error(PipelineErrorVariant::GetUnprocessableQueryParams(e))
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
