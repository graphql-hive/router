use std::sync::Arc;

use crate::pipeline::error::{PipelineError, PipelineErrorVariant};
use crate::pipeline::gateway_layer::{
    GatewayPipelineLayer, GatewayPipelineStepDecision, ProcessorLayer,
};
use crate::pipeline::http_request_params::HttpRequestParams;
use crate::pipeline::parser_service::GraphQLParserPayload;
use crate::shared_state::GatewaySharedState;
use axum::body::Body;
use graphql_tools::validation::validate::validate;
use http::Request;
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
        req: &mut Request<Body>,
    ) -> Result<GatewayPipelineStepDecision, PipelineError> {
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

        let consumer_schema_ast = &app_state.planner.consumer_schema.document;

        let validation_result = match app_state
            .validate_cache
            .get(&parser_payload.cache_key)
            .await
        {
            Some(cached_validation) => {
                trace!(
                    "validation result of hash {} has been loaded from cache",
                    parser_payload.cache_key
                );

                cached_validation
            }
            None => {
                trace!(
                    "validation result of hash {} does not exists in cache",
                    parser_payload.cache_key
                );

                let res = validate(
                    consumer_schema_ast,
                    &parser_payload.parsed_operation,
                    &app_state.validation_plan,
                );
                let arc_res = Arc::new(res);

                app_state
                    .validate_cache
                    .insert(parser_payload.cache_key, arc_res.clone())
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

            let http_payload = req.extensions().get::<HttpRequestParams>().ok_or_else(|| {
                PipelineErrorVariant::InternalServiceError("HttpRequestParams is missing")
            })?;

            return Err(PipelineError::new_with_accept_header(
                PipelineErrorVariant::ValidationErrors(validation_result),
                http_payload.accept_header.clone(),
            ));
        }

        Ok(GatewayPipelineStepDecision::Continue)
    }
}
