use std::sync::Arc;

use crate::pipeline::error::{PipelineError, PipelineErrorVariant};
use crate::pipeline::gateway_layer::{
    GatewayPipelineLayer, GatewayPipelineStepDecision, ProcessorLayer,
};
use crate::pipeline::http_request_params::HttpRequestParams;
use crate::pipeline::normalize_service::GraphQLNormalizationPayload;
use crate::pipeline::parser_service::GraphQLParserPayload;
use crate::shared_state::GatewaySharedState;
use axum::body::Body;
use graphql_tools::validation::validate::validate;
use http::{Method, Request};
use query_planner::state::supergraph_state::OperationKind;
use tracing::{error, trace};

#[derive(Clone, Debug, Default)]
pub struct GraphQLValidationService;

impl GraphQLValidationService {
    pub fn new_layer() -> ProcessorLayer<Self> {
        ProcessorLayer::new(Self)
    }
}

#[async_trait::async_trait]
impl GatewayPipelineLayer for GraphQLValidationService {
    #[tracing::instrument(level = "trace", name = "GraphQLValidationService", skip_all)]
    async fn process(
        &self,
        req: Request<Body>,
    ) -> Result<(Request<Body>, GatewayPipelineStepDecision), PipelineError> {
        let normalized_operation = req
            .extensions()
            .get::<GraphQLNormalizationPayload>()
            .ok_or_else(|| {
                PipelineErrorVariant::InternalServiceError("GraphQLNormalizationPayload is missing")
            })?;

        let http_params = req.extensions().get::<HttpRequestParams>().ok_or_else(|| {
            PipelineErrorVariant::InternalServiceError("HttpRequestParams is missing")
        })?;

        let parser_payload = req
            .extensions()
            .get::<GraphQLParserPayload>()
            .ok_or_else(|| {
                PipelineErrorVariant::InternalServiceError("GraphQLParserPayload is missing")
            })?;

        let app_state = req
            .extensions()
            .get::<Arc<GatewaySharedState>>()
            .ok_or_else(|| {
                PipelineErrorVariant::InternalServiceError("GatewaySharedState is missing")
            })?;

        if http_params.http_method == Method::GET {
            if let Some(OperationKind::Mutation) = normalized_operation
                .normalized_document
                .operation
                .operation_kind
            {
                error!("Mutation is not allowed over GET, stopping");

                return Err(PipelineErrorVariant::MutationNotAllowedOverHttpGet.into());
            }
        }

        let consumer_schema_ast = &app_state.planner.consumer_schema.document;
        let validation_cache_key = normalized_operation.normalized_document.operation.hash();

        let validation_result = match app_state.validate_cache.get(&validation_cache_key).await {
            Some(cached_validation) => {
                trace!(
                    "validation result of hash {} has been loaded from cache",
                    validation_cache_key
                );

                cached_validation
            }
            None => {
                trace!(
                    "validation result of hash {} does not exists in cache",
                    validation_cache_key
                );

                let res = validate(
                    consumer_schema_ast,
                    &parser_payload.parsed_operation,
                    &app_state.validation_plan,
                );
                let arc_res = Arc::new(res);

                app_state
                    .validate_cache
                    .insert(validation_cache_key, arc_res.clone())
                    .await;
                arc_res
            }
        };

        if !validation_result.is_empty() {
            error!(
                "GraphQL validation failed with total of {} errors",
                validation_result.len()
            );
            trace!("Validation errors: {:?}", validation_result);

            return Err(PipelineErrorVariant::ValidationErrors(validation_result).into());
        }

        Ok((req, GatewayPipelineStepDecision::Continue))
    }
}
