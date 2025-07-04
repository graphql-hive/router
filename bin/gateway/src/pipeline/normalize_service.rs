use std::sync::Arc;

use axum::body::Body;
use http::Request;
use query_plan_executor::introspection::filter_introspection_fields_in_operation;
use query_planner::ast::document::NormalizedDocument;
use query_planner::ast::normalization::normalize_operation;
use query_planner::ast::operation::OperationDefinition;

use crate::pipeline::error::{PipelineError, PipelineErrorVariant};
use crate::pipeline::gateway_layer::{
    GatewayPipelineLayer, GatewayPipelineStepDecision, ProcessorLayer,
};
use crate::pipeline::graphql_request_params::ExecutionRequest;
use crate::pipeline::http_request_params::HttpRequestParams;
use crate::pipeline::parser_service::GraphQLParserPayload;
use crate::shared_state::GatewaySharedState;
use tracing::{error, trace};

#[derive(Debug, Clone)]
pub struct GraphQLNormalizationPayload {
    /// The raw, normalized GraphQL document.
    pub normalized_document: Arc<NormalizedDocument>,
    /// The operation to execute, without introspection fields.
    pub operation_for_plan: OperationDefinition,
    pub has_introspection: bool,
}

#[derive(Clone, Debug, Default)]
pub struct GraphQLOperationNormalizationService;

impl GraphQLOperationNormalizationService {
    pub fn new_layer() -> ProcessorLayer<Self> {
        ProcessorLayer::new(Self)
    }
}

#[async_trait::async_trait]
impl GatewayPipelineLayer for GraphQLOperationNormalizationService {
    #[tracing::instrument(
        level = "trace",
        name = "GraphQLOperationNormalizationService",
        skip_all
    )]
    async fn process(
        &self,
        mut req: Request<Body>,
    ) -> Result<(Request<Body>, GatewayPipelineStepDecision), PipelineError> {
        let parser_payload = req
            .extensions()
            .get::<GraphQLParserPayload>()
            .ok_or_else(|| {
                PipelineErrorVariant::InternalServiceError("GraphQLParserPayload is missing")
            })?;
        let http_payload = req.extensions().get::<HttpRequestParams>().ok_or_else(|| {
            PipelineErrorVariant::InternalServiceError("HttpRequestParams is missing")
        })?;

        let execution_params = req.extensions().get::<ExecutionRequest>().ok_or_else(|| {
            PipelineErrorVariant::InternalServiceError("ExecutionRequest is missing")
        })?;

        let app_state = req
            .extensions()
            .get::<Arc<GatewaySharedState>>()
            .ok_or_else(|| {
                PipelineErrorVariant::InternalServiceError("GatewaySharedState is missing")
            })?;

        let normalized_document = match app_state.normalize_cache.get(&execution_params.query).await
        {
            Some(normalized_document) => {
                trace!("Found cached normalized document for operation");

                normalized_document
            }
            None => match normalize_operation(
                &app_state.planner.supergraph,
                &parser_payload.parsed_operation,
                execution_params.operation_name.as_deref(),
            ) {
                Ok(normalized_document) => {
                    trace!("Successfully normalized GraphQL operation");

                    let normalized_document = Arc::new(normalized_document);
                    app_state
                        .normalize_cache
                        .insert(
                            execution_params.query.to_string(),
                            normalized_document.clone(),
                        )
                        .await;

                    normalized_document
                }
                Err(err) => {
                    error!("Failed to normalize GraphQL operation: {}", err);
                    trace!("{:?}", err);

                    return Err(PipelineError::new_with_accept_header(
                        PipelineErrorVariant::NormalizationError(err),
                        http_payload.accept_header.clone(),
                    ));
                }
            },
        };

        trace!(
            "Successfully normalized GraphQL operation (operation name={:?}): {}",
            normalized_document.operation_name,
            normalized_document.operation
        );

        let operation = &normalized_document.operation;
        let (has_introspection, operation_for_plan) =
            filter_introspection_fields_in_operation(operation);

        trace!(
            "Operation after removing introspection fields (introspection found={}): {}",
            has_introspection,
            operation_for_plan
        );

        req.extensions_mut().insert(GraphQLNormalizationPayload {
            normalized_document,
            operation_for_plan,
            has_introspection,
        });

        Ok((req, GatewayPipelineStepDecision::Continue))
    }
}
