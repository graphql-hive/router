use axum::body::Body;
use http::header::{ACCEPT, CONTENT_TYPE};
use http::{HeaderValue, Method, Request};
use lazy_static::lazy_static;
use tracing::trace;

use crate::pipeline::error::{PipelineError, PipelineErrorVariant};
use crate::pipeline::gateway_layer::{
    GatewayPipelineLayer, GatewayPipelineStepDecision, ProcessorLayer,
};

lazy_static! {
    pub static ref APPLICATION_JSON: HeaderValue = HeaderValue::from_static("application/json");
    pub static ref APPLICATION_GRAPHQL_RESPONSE_JSON: HeaderValue =
        HeaderValue::from_static("application/graphql-response+json");
}

#[derive(Debug, Clone)]
pub struct HttpRequestParams {
    pub accept_header: String,
    pub http_method: Method,
    pub request_content_type: Option<String>,
    pub response_content_type: HeaderValue,
}

#[derive(Clone, Debug, Default)]
pub struct HttpRequestParamsExtractor;

impl HttpRequestParamsExtractor {
    pub fn new_layer() -> ProcessorLayer<Self> {
        ProcessorLayer::new(Self)
    }
}

#[async_trait::async_trait]
impl GatewayPipelineLayer for HttpRequestParamsExtractor {
    async fn process(
        &self,
        mut req: Request<Body>,
    ) -> Result<(Request<Body>, GatewayPipelineStepDecision), PipelineError> {
        let http_method = req.method().to_owned();
        let accept_header = req
            .headers()
            .get(ACCEPT)
            .unwrap_or(&APPLICATION_JSON)
            .to_str()
            .map_err(|_| PipelineErrorVariant::InvalidHeaderValue(ACCEPT.to_string()))?
            .to_owned();

        trace!(
            "Using the following Accept header for request: {}",
            &accept_header
        );

        let request_content_type: Option<String> = match req.headers().get(CONTENT_TYPE) {
            None => None,
            Some(content_type) => {
                let value = content_type
                    .to_str()
                    .map_err(|_| PipelineErrorVariant::InvalidHeaderValue(ACCEPT.to_string()))?
                    .to_owned();

                Some(value)
            }
        };

        trace!(
            "Using the following Content-Type header for request: {:?}",
            &request_content_type
        );

        let response_content_type =
            if accept_header.contains(APPLICATION_GRAPHQL_RESPONSE_JSON.to_str().unwrap()) {
                APPLICATION_GRAPHQL_RESPONSE_JSON.clone()
            } else {
                APPLICATION_JSON.clone()
            };

        trace!(
            "Will use the following Content-Type header for response: {:?}",
            &response_content_type
        );

        let extracted_params = HttpRequestParams {
            http_method,
            accept_header,
            response_content_type,
            request_content_type,
        };

        trace!("Extracted HTTP params: {:?}", extracted_params);

        req.extensions_mut().insert(extracted_params);

        Ok((req, GatewayPipelineStepDecision::Continue))
    }
}
