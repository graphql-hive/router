use axum::body::Body;
use axum::response::IntoResponse;
use http::{Method, Request};

use crate::http_utils::landing_page::PRODUCT_LOGO_SVG;
use crate::pipeline::error::PipelineError;
use crate::pipeline::gateway_layer::{
    GatewayPipelineLayer, GatewayPipelineStepDecision, ProcessorLayer,
};
use crate::pipeline::header::RequestAccepts;

use axum::response::Html;

static GRAPHIQL_HTML: &str = include_str!("../../static/graphiql.html");

#[derive(Clone, Debug, Default)]
pub struct GraphiQLResponderService;

#[async_trait::async_trait]
impl GatewayPipelineLayer for GraphiQLResponderService {
    async fn process(
        &self,
        req: &mut Request<Body>,
    ) -> Result<GatewayPipelineStepDecision, PipelineError> {
        if req.method() == Method::GET && req.accepts_content_type("text/html") {
            return Ok(GatewayPipelineStepDecision::RespondWith(
                Html(GRAPHIQL_HTML.replace("__PRODUCT_LOGO__", PRODUCT_LOGO_SVG)).into_response(),
            ));
        }

        Ok(GatewayPipelineStepDecision::Continue)
    }
}

impl GraphiQLResponderService {
    pub fn new_layer() -> ProcessorLayer<Self> {
        ProcessorLayer::new(Self)
    }
}
