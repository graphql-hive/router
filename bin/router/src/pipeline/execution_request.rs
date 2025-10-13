use std::collections::HashMap;

use hive_router_plan_executor::hooks::on_graphql_params::{
    GraphQLParams, OnGraphQLParamsEndHookPayload, OnGraphQLParamsStartHookPayload,
};
use hive_router_plan_executor::plugin_context::PluginRequestState;
use hive_router_plan_executor::plugin_trait::{EndControlFlow, StartControlFlow};
use http::{header::CONTENT_TYPE, Method};
use ntex::util::Bytes;
use ntex::web::types::Query;
use ntex::web::HttpRequest;
use tracing::{trace, warn};

use crate::pipeline::error::PipelineError;
use crate::pipeline::header::SingleContentType;

#[derive(serde::Deserialize, Debug)]
struct GETQueryParams {
    pub query: Option<String>,
    #[serde(rename = "camelCase")]
    pub operation_name: Option<String>,
    pub variables: Option<String>,
    pub extensions: Option<String>,
}

impl TryInto<GraphQLParams> for GETQueryParams {
    type Error = PipelineError;

    fn try_into(self) -> Result<GraphQLParams, Self::Error> {
        let variables = match self.variables.as_deref() {
            Some(v_str) if !v_str.is_empty() => match sonic_rs::from_str(v_str) {
                Ok(vars) => vars,
                Err(e) => {
                    return Err(PipelineError::FailedToParseVariables(e));
                }
            },
            _ => HashMap::new(),
        };

        let extensions = match self.extensions.as_deref() {
            Some(e_str) if !e_str.is_empty() => match sonic_rs::from_str(e_str) {
                Ok(exts) => Some(exts),
                Err(e) => {
                    return Err(PipelineError::FailedToParseExtensions(e));
                }
            },
            _ => None,
        };

        let execution_request = GraphQLParams {
            query: self.query,
            operation_name: self.operation_name,
            variables,
            extensions,
        };

        Ok(execution_request)
    }
}

pub trait GetQueryStr {
    fn get_query(&self) -> Result<&str, PipelineError>;
}

impl GetQueryStr for GraphQLParams {
    fn get_query(&self) -> Result<&str, PipelineError> {
        self.query
            .as_deref()
            .ok_or(PipelineError::GetMissingQueryParam("query"))
    }
}

pub enum DeserializationResult {
    EarlyResponse(ntex::web::HttpResponse),
    GraphQLParams(GraphQLParams),
}

#[inline]
pub async fn deserialize_graphql_params(
    req: &HttpRequest,
    body: Bytes,
    plugin_req_state: &Option<PluginRequestState<'_>>,
) -> Result<DeserializationResult, PipelineError> {
    /* Handle on_deserialize hook in the plugins - START */
    let mut deserialization_end_callbacks = vec![];

    let mut graphql_params = None;
    let mut body = body;
    if let Some(plugin_req_state) = plugin_req_state.as_ref() {
        let mut deserialization_payload: OnGraphQLParamsStartHookPayload =
            OnGraphQLParamsStartHookPayload {
                router_http_request: &plugin_req_state.router_http_request,
                context: &plugin_req_state.context,
                body,
                graphql_params: None,
            };
        for plugin in plugin_req_state.plugins.as_ref() {
            let result = plugin.on_graphql_params(deserialization_payload).await;
            deserialization_payload = result.payload;
            match result.control_flow {
                StartControlFlow::Proceed => { /* continue to next plugin */ }
                StartControlFlow::EndWithResponse(response) => {
                    return Ok(DeserializationResult::EarlyResponse(response));
                }
                StartControlFlow::OnEnd(callback) => {
                    deserialization_end_callbacks.push(callback);
                }
            }
        }
        // Give the ownership back to variables
        graphql_params = deserialization_payload.graphql_params;
        body = deserialization_payload.body;
    }

    let mut graphql_params = match graphql_params {
        Some(params) => params,
        None => {
            let http_method = req.method();
            match *http_method {
                Method::GET => {
                    trace!("processing GET GraphQL operation");
                    let query_params_str = req
                        .uri()
                        .query()
                        .ok_or_else(|| PipelineError::GetInvalidQueryParams)?;
                    let query_params = Query::<GETQueryParams>::from_query(query_params_str)?.0;

                    trace!("parsed GET query params: {:?}", query_params);

                    query_params.try_into()?
                }
                Method::POST => {
                    trace!("Processing POST GraphQL request");

                    match req.headers().get(CONTENT_TYPE) {
                        Some(value) => {
                            let content_type_str = value
                                .to_str()
                                .map_err(|_| PipelineError::InvalidHeaderValue(CONTENT_TYPE))?;
                            if !content_type_str.contains(SingleContentType::JSON.as_ref()) {
                                warn!(
                                    "Invalid content type on a POST request: {}",
                                    content_type_str
                                );
                                return Err(PipelineError::UnsupportedContentType);
                            }
                        }
                        None => {
                            trace!("POST without content type detected");
                            return Err(PipelineError::MissingContentTypeHeader);
                        }
                    }

                    let execution_request = unsafe {
                        sonic_rs::from_slice_unchecked::<GraphQLParams>(&body).map_err(|e| {
                            warn!("Failed to parse body: {}", e);
                            PipelineError::FailedToParseBody(e)
                        })?
                    };

                    execution_request
                }
                _ => {
                    warn!("unsupported HTTP method: {}", http_method);

                    return Err(PipelineError::UnsupportedHttpMethod(http_method.to_owned()));
                }
            }
        }
    };

    if let Some(plugin_req_state) = &plugin_req_state {
        let mut payload = OnGraphQLParamsEndHookPayload {
            graphql_params,
            context: &plugin_req_state.context,
        };
        for deserialization_end_callback in deserialization_end_callbacks {
            let result = deserialization_end_callback(payload);
            payload = result.payload;
            match result.control_flow {
                EndControlFlow::Proceed => { /* continue to next plugin */ }
                EndControlFlow::EndWithResponse(response) => {
                    return Ok(DeserializationResult::EarlyResponse(response));
                }
            }
        }
        graphql_params = payload.graphql_params;
    }

    /* Handle on_deserialize hook in the plugins - END */

    Ok(DeserializationResult::GraphQLParams(graphql_params))
}
