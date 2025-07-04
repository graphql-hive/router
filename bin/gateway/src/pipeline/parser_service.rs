use std::sync::Arc;

use axum::body::Body;
use graphql_parser::query::Document;
use http::Request;
use query_planner::utils::parsing::safe_parse_operation;

use crate::pipeline::error::{PipelineError, PipelineErrorVariant};
use crate::pipeline::gateway_layer::{
    GatewayPipelineLayer, GatewayPipelineStepDecision, ProcessorLayer,
};
use crate::pipeline::graphql_request_params::ExecutionRequest;
use crate::pipeline::http_request_params::HttpRequestParams;
use crate::shared_state::GatewaySharedState;
use tracing::{error, trace};

#[derive(Debug, Clone)]
pub struct GraphQLParserPayload {
    pub parsed_operation: Arc<Document<'static, String>>,
}

#[derive(Clone, Debug, Default)]
pub struct GraphQLParserService;

impl GraphQLParserService {
    pub fn new_layer() -> ProcessorLayer<Self> {
        ProcessorLayer::new(Self)
    }
}

#[async_trait::async_trait]
impl GatewayPipelineLayer for GraphQLParserService {
    #[tracing::instrument(level = "trace", name = "GraphQLParserService", skip_all)]
    async fn process(
        &self,
        mut req: Request<Body>,
    ) -> Result<(Request<Body>, GatewayPipelineStepDecision), PipelineError> {
        let execution_params = req.extensions().get::<ExecutionRequest>().ok_or_else(|| {
            PipelineErrorVariant::InternalServiceError("ExecutionRequest is missing")
        })?;
        let http_params = req.extensions().get::<HttpRequestParams>().ok_or_else(|| {
            PipelineErrorVariant::InternalServiceError("HttpRequestParams is missing")
        })?;

        let app_state = req
            .extensions()
            .get::<Arc<GatewaySharedState>>()
            .ok_or_else(|| {
                PipelineErrorVariant::InternalServiceError("GatewaySharedState is missing")
            })?;

        match app_state.parse_cache.get(&execution_params.query).await {
            Some(parsed_operation) => {
                trace!("Found cached parsed operation for query");

                req.extensions_mut()
                    .insert(GraphQLParserPayload { parsed_operation });

                return Ok((req, GatewayPipelineStepDecision::Continue));
            }
            None => match safe_parse_operation(&execution_params.query) {
                Ok(parsed_operation) => {
                    trace!("sucessfully parsed GraphQL operation");
                    let parsed_operation = Arc::new(parsed_operation);
                    app_state
                        .parse_cache
                        .insert(execution_params.query.to_string(), parsed_operation.clone())
                        .await;
                    req.extensions_mut()
                        .insert(GraphQLParserPayload { parsed_operation });

                    Ok((req, GatewayPipelineStepDecision::Continue))
                }
                Err(err) => {
                    error!("Failed to parse GraphQL operation: {}", err);

                    Err(PipelineError::new_with_accept_header(
                        PipelineErrorVariant::FailedToParseOperation(err),
                        http_params.accept_header.clone(),
                    ))
                }
            },
        }
    }
}
