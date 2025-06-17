use axum::body::Body;
use axum::response::IntoResponse;
use http::{Method, Request};

use crate::pipeline::error::{PipelineError, PipelineErrorVariant};
use crate::pipeline::gateway_layer::{
    GatewayPipelineLayer, GatewayPipelineStepDecision, ProcessorLayer,
};
use crate::pipeline::http_request_params::HttpRequestParams;

use axum::response::Html;

static GRAPHIQL_HTML: &str = include_str!("../../static/graphiql.html");

#[derive(Clone, Debug, Default)]
pub struct GraphiQLResponderService;

#[async_trait::async_trait]
impl GatewayPipelineLayer for GraphiQLResponderService {
    async fn process(
        &self,
        req: Request<Body>,
    ) -> Result<(Request<Body>, GatewayPipelineStepDecision), PipelineError> {
        let http_params = req.extensions().get::<HttpRequestParams>().ok_or_else(|| {
            PipelineErrorVariant::InternalServiceError("HttpRequestParams is missing")
        })?;

        if http_params.http_method == Method::GET && http_params.accept_header.contains("text/html")
        {
            return Ok((
                req,
                GatewayPipelineStepDecision::RespondWith(Html(GRAPHIQL_HTML).into_response()),
            ));
        }

        Ok((req, GatewayPipelineStepDecision::Continue))
    }
}

impl GraphiQLResponderService {
    pub fn new_layer() -> ProcessorLayer<Self> {
        ProcessorLayer::new(Self)
    }
}
