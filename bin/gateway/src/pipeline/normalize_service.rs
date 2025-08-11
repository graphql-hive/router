use std::hash::{Hash, Hasher};
use std::sync::Arc;

use axum::body::Body;
use executor::introspection::partition::partition_operation;
use http::Request;
use query_plan_executor::projection::FieldProjectionPlan;
use query_planner::ast::normalization::normalize_operation;
use query_planner::ast::operation::OperationDefinition;
use xxhash_rust::xxh3::Xxh3;

use crate::pipeline::error::{PipelineError, PipelineErrorFromAcceptHeader, PipelineErrorVariant};
use crate::pipeline::gateway_layer::{
    GatewayPipelineLayer, GatewayPipelineStepDecision, ProcessorLayer,
};
use crate::pipeline::graphql_request_params::ExecutionRequest;
use crate::pipeline::parser_service::GraphQLParserPayload;
use crate::shared_state::GatewaySharedState;
use tracing::{error, trace};

#[derive(Debug)]
pub struct GraphQLNormalizationPayload {
    /// The operation to execute, without introspection fields.
    pub operation_for_plan: OperationDefinition,
    pub operation_for_introspection: Option<OperationDefinition>,
    pub root_type_name: &'static str,
    pub projection_plan: Vec<FieldProjectionPlan>,
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
        req: &mut Request<Body>,
    ) -> Result<GatewayPipelineStepDecision, PipelineError> {
        let parser_payload = req
            .extensions()
            .get::<GraphQLParserPayload>()
            .ok_or_else(|| {
                req.new_pipeline_error(PipelineErrorVariant::InternalServiceError(
                    "GraphQLParserPayload is missing",
                ))
            })?;

        let execution_params = req.extensions().get::<ExecutionRequest>().ok_or_else(|| {
            req.new_pipeline_error(PipelineErrorVariant::InternalServiceError(
                "ExecutionRequest is missing",
            ))
        })?;

        let app_state = req
            .extensions()
            .get::<Arc<GatewaySharedState>>()
            .ok_or_else(|| {
                req.new_pipeline_error(PipelineErrorVariant::InternalServiceError(
                    "GatewaySharedState is missing",
                ))
            })?;

        let cache_key = match &execution_params.operation_name {
            Some(operation_name) => {
                let mut hasher = Xxh3::new();
                execution_params.query.hash(&mut hasher);
                operation_name.hash(&mut hasher);
                hasher.finish()
            }
            None => parser_payload.cache_key,
        };

        match app_state.normalize_cache.get(&cache_key).await {
            Some(payload) => {
                trace!(
                    "Found normalized GraphQL operation in cache (operation name={:?}): {}",
                    payload.operation_for_plan.name,
                    payload.operation_for_plan
                );
                req.extensions_mut().insert(payload);
                Ok(GatewayPipelineStepDecision::Continue)
            }
            None => match normalize_operation(
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

                    let operation = doc.operation;
                    let (root_type_name, projection_plan) =
                        FieldProjectionPlan::from_operation(&operation, &app_state.schema_metadata);
                    let partitioned_operation = partition_operation(operation);
                    // let (has_introspection, filtered_operation_for_plan) =
                    //     filter_introspection_fields_in_operation(operation);

                    // trace!(
                    //     "Operation after removing introspection fields (introspection found={}): {}",
                    //     has_introspection,
                    //     filtered_operation_for_plan
                    // );

                    let payload = GraphQLNormalizationPayload {
                        root_type_name,
                        projection_plan,
                        operation_for_plan: partitioned_operation.downstream_operation,
                        operation_for_introspection: partitioned_operation.introspection_operation,
                    };
                    let payload_arc = Arc::new(payload);
                    app_state
                        .normalize_cache
                        .insert(cache_key, payload_arc.clone())
                        .await;
                    req.extensions_mut().insert(payload_arc);
                    Ok(GatewayPipelineStepDecision::Continue)
                }
                Err(err) => {
                    error!("Failed to normalize GraphQL operation: {}", err);
                    trace!("{:?}", err);

                    Err(req.new_pipeline_error(PipelineErrorVariant::NormalizationError(err)))
                }
            },
        }
    }
}
