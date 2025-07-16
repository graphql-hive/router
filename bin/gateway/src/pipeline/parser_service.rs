use std::hash::{DefaultHasher, Hash, Hasher};
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
    pub cache_key: u64,
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
        req: &mut Request<Body>,
    ) -> Result<GatewayPipelineStepDecision, PipelineError> {
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

        let cache_key = {
            let mut hasher = DefaultHasher::new();
            execution_params.query.hash(&mut hasher);
            hasher.finish()
        };

        let parsed_operation = if let Some(cached) = app_state.parse_cache.get(&cache_key).await {
            trace!("Found cached parsed operation for query");
            cached
        } else {
            let parsed = safe_parse_operation(&execution_params.query).map_err(|err| {
                error!("Failed to parse GraphQL operation: {}", err);
                PipelineError::new_with_accept_header(
                    PipelineErrorVariant::FailedToParseOperation(err),
                    http_params.accept_header.clone(),
                )
            })?;
            trace!("sucessfully parsed GraphQL operation");
            let parsed_arc = Arc::new(parsed);
            app_state
                .parse_cache
                .insert(cache_key, parsed_arc.clone())
                .await;
            parsed_arc
        };

        req.extensions_mut().insert(GraphQLParserPayload {
            parsed_operation,
            cache_key,
        });

        Ok(GatewayPipelineStepDecision::Continue)
    }
}
