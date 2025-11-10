use core::fmt;
use std::collections::HashMap;

use http::Method;
use ntex::util::Bytes;
use ntex::web::types::Query;
use ntex::web::HttpRequest;
use serde::{de, Deserialize, Deserializer};
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
    #[serde(flatten)]
    pub extra_params: HashMap<String, Value>,
}

#[derive(Debug, Clone)]
pub struct ExecutionRequest {
    pub query: Option<String>,
    pub operation_name: Option<String>,
    pub variables: HashMap<String, Value>,
    #[allow(dead_code)]
    pub extensions: Option<HashMap<String, Value>>,
    pub extra_params: HashMap<String, Value>,
}

// Workaround for https://github.com/cloudwego/sonic-rs/issues/114

impl<'de> Deserialize<'de> for ExecutionRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct GraphQLErrorExtensionsVisitor;

        impl<'de> de::Visitor<'de> for GraphQLErrorExtensionsVisitor {
            type Value = ExecutionRequest;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a map for GraphQLErrorExtensions")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: de::MapAccess<'de>,
            {
                let mut query = None;
                let mut operation_name = None;
                let mut variables: Option<HashMap<String, Value>> = None;
                let mut extensions: Option<HashMap<String, Value>> = None;
                let mut extra_params = HashMap::new();

                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "query" => {
                            if query.is_some() {
                                return Err(de::Error::duplicate_field("query"));
                            }
                            query = Some(map.next_value()?);
                        }
                        "operationName" => {
                            if operation_name.is_some() {
                                return Err(de::Error::duplicate_field("operationName"));
                            }
                            operation_name = Some(map.next_value()?);
                        }
                        "variables" => {
                            if variables.is_some() {
                                return Err(de::Error::duplicate_field("variables"));
                            }
                            // Handle if variables do not exist or null
                            if let Some(vars) =
                                map.next_value::<Option<HashMap<String, Value>>>()?
                            {
                                variables = Some(vars);
                            } else {
                                variables = Some(HashMap::new());
                            }
                        }
                        "extensions" => {
                            if extensions.is_some() {
                                return Err(de::Error::duplicate_field("extensions"));
                            }
                            extensions = Some(map.next_value()?);
                        }
                        other => {
                            let value: Value = map.next_value()?;
                            extra_params.insert(other.to_string(), value);
                        }
                    }
                }

                Ok(ExecutionRequest {
                    query,
                    operation_name,
                    variables: variables.unwrap_or_default(),
                    extensions,
                    extra_params,
                })
            }
        }

        deserializer.deserialize_map(GraphQLErrorExtensionsVisitor)
    }
}

impl TryInto<ExecutionRequest> for GETQueryParams {
    type Error = PipelineErrorVariant;

    fn try_into(self) -> Result<ExecutionRequest, Self::Error> {
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
            query: self.query,
            operation_name: self.operation_name,
            variables,
            extensions,
            extra_params: self.extra_params,
        };

        Ok(execution_request)
    }
}

#[inline]
pub async fn get_execution_request(
    req: &mut HttpRequest,
    body_bytes: Bytes,
) -> Result<ExecutionRequest, PipelineError> {
    let http_method = req.method();
    let execution_request: ExecutionRequest = match *http_method {
        Method::GET => {
            trace!("processing GET GraphQL operation");
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

impl ExecutionRequest {
    pub fn get_query_str(&self) -> Result<&str, PipelineErrorVariant> {
        match &self.query {
            Some(query_str) => Ok(query_str.as_str()),
            None => Err(PipelineErrorVariant::GetMissingQueryParam("query")),
        }
    }
}
