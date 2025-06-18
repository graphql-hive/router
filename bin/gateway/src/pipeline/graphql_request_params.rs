use axum::body::{to_bytes, Body};
use axum::extract::Query;
use http::{Method, Request};
use query_plan_executor::ExecutionRequest;
use tracing::{trace, warn};

use crate::pipeline::error::{PipelineError, PipelineErrorVariant};
use crate::pipeline::gateway_layer::{
    GatewayPipelineLayer, GatewayPipelineStepDecision, ProcessorLayer,
};
use crate::pipeline::http_request_params::{HttpRequestParams, APPLICATION_JSON};

static MAX_BODY_SIZE: usize = 2 * 1024 * 1024; // 2 MB in bytes, like Axum's default

#[derive(Clone, Debug, Default)]
pub struct GraphQLRequestParamsExtractor;

#[derive(serde::Deserialize, Debug)]
struct GETQueryParams {
    pub query: Option<String>,
    #[serde(rename = "camelCase")]
    pub operation_name: Option<String>,
    pub variables: Option<String>,
    pub extensions: Option<String>,
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

#[async_trait::async_trait]
impl GatewayPipelineLayer for GraphQLRequestParamsExtractor {
    #[tracing::instrument(level = "debug", name = "GraphQLRequestParamsExtractor", skip_all)]
    async fn process(
        &self,
        mut req: Request<Body>,
    ) -> Result<(Request<Body>, GatewayPipelineStepDecision), PipelineError> {
        let http_params = req.extensions().get::<HttpRequestParams>().ok_or_else(|| {
            PipelineErrorVariant::InternalServiceError("HttpRequestParams is missing")
        })?;

        let accept_header = http_params.accept_header.clone();
        let execution_request: ExecutionRequest = match http_params.http_method {
            Method::GET => {
                trace!("processing GET GraphQL operation");

                let query_params = Query::<GETQueryParams>::try_from_uri(req.uri())
                    .map_err(|qe| {
                        PipelineError::new_with_accept_header(
                            PipelineErrorVariant::GetInvalidQueryParams(qe),
                            accept_header.clone(),
                        )
                    })?
                    .0;

                trace!("parsed GET query params: {:?}", query_params);

                query_params.try_into()?
            }
            Method::POST => {
                trace!("Processing POST GraphQL request");

                match &http_params.request_content_type {
                    None => {
                        trace!("POST without content type detected");

                        return Err(PipelineError::new_with_accept_header(
                            PipelineErrorVariant::MissingContentTypeHeader,
                            accept_header.clone(),
                        ));
                    }
                    Some(content_type) => {
                        if !content_type.contains(APPLICATION_JSON.to_str().unwrap()) {
                            warn!("Invalid content type on a POST request: {}", content_type);

                            return Err(PipelineError::new_with_accept_header(
                                PipelineErrorVariant::UnsupportedContentType,
                                accept_header.clone(),
                            ));
                        }
                    }
                }

                let (mut parts, body) = req.into_parts();
                let body_bytes = to_bytes(body, MAX_BODY_SIZE).await.map_err(|err| {
                    warn!("Failed to read body bytes: {}", err);

                    PipelineError::new_with_accept_header(
                        PipelineErrorVariant::FailedToReadBodyBytes(err),
                        accept_header.clone(),
                    )
                })?;

                let execution_request = sonic_rs::from_slice::<ExecutionRequest>(&body_bytes)
                    .map_err(|e| {
                        warn!("Failed to parse body: {}", e);

                        PipelineError::new_with_accept_header(
                            PipelineErrorVariant::FailedToParseBody(e),
                            accept_header,
                        )
                    })?;

                trace!("Body is parsed, will proceed");
                parts.extensions.insert(execution_request.clone());
                req = Request::from_parts(parts, Body::from(body_bytes));

                execution_request
            }
            _ => {
                warn!("unsupported HTTP method: {}", http_params.http_method);

                return Err(PipelineErrorVariant::UnsupportedHttpMethod(
                    http_params.http_method.to_string(),
                )
                .into());
            }
        };

        req.extensions_mut().insert(execution_request);

        Ok((req, GatewayPipelineStepDecision::Continue))
    }
}

impl GraphQLRequestParamsExtractor {
    pub fn new_layer() -> ProcessorLayer<Self> {
        ProcessorLayer::new(Self)
    }
}
