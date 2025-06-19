use std::sync::Arc;

use axum::body::Body;
use http::Request;
use query_plan_executor::introspection::filter_introspection_fields_in_operation;
use query_plan_executor::ExecutionRequest;
use query_planner::ast::document::NormalizedDocument;
use query_planner::ast::normalization::normalize_operation;
use query_planner::ast::operation::OperationDefinition;

use crate::pipeline::error::{PipelineError, PipelineErrorVariant};
use crate::pipeline::gateway_layer::{
    GatewayPipelineLayer, GatewayPipelineStepDecision, ProcessorLayer,
};
use crate::pipeline::http_request_params::HttpRequestParams;
use crate::pipeline::parser_service::GraphQLParserPayload;
use crate::shared_state::GatewaySharedState;
use tracing::{error, trace};

#[derive(Debug, Clone)]
pub struct GraphQLNormalizationPayload {
    /// The raw, normalized GraphQL document.
    pub normalized_document: NormalizedDocument,
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

        match normalize_operation(
            &app_state.planner.supergraph,
            &parser_payload.parsed_operation,
            execution_params.operation_name.as_deref(),
        ) {
            Ok(doc) => {
                trace!(
                    "Successfully normalized GraphQL operation (operation name={:?}): {}",
                    doc.operation_name,
                    doc.operation
                );

                let operation = &doc.operation;
                let (has_introspection, filtered_operation_for_plan) =
                    filter_introspection_fields_in_operation(operation);

                trace!(
                    "Operation after removing introspection fields (introspection found={}): {}",
                    has_introspection,
                    filtered_operation_for_plan
                );

                req.extensions_mut().insert(GraphQLNormalizationPayload {
                    normalized_document: doc,
                    operation_for_plan: filtered_operation_for_plan,
                    has_introspection,
                });

                Ok((req, GatewayPipelineStepDecision::Continue))
            }
            Err(err) => {
                error!("Failed to normalize GraphQL operation: {}", err);
                trace!("{:?}", err);

                return Err(PipelineError::new_with_accept_header(
                    PipelineErrorVariant::NormalizationError(err),
                    http_payload.accept_header.clone(),
                ));
            }
        }
    }
}
