use std::hash::{Hash, Hasher};
use std::sync::Arc;

use executor::introspection::partition::partition_operation;
use executor::projection::plan::FieldProjectionPlan;
use ntex::web::HttpRequest;
use query_planner::ast::normalization::normalize_operation;
use query_planner::ast::operation::OperationDefinition;
use xxhash_rust::xxh3::Xxh3;

use crate::pipeline::error::{PipelineError, PipelineErrorFromAcceptHeader, PipelineErrorVariant};
use crate::pipeline::graphql_request_params::ExecutionRequest;
use crate::pipeline::parser_service::GraphQLParserPayload;
use crate::shared_state::GatewaySharedState;
use tracing::{error, trace};

#[derive(Debug)]
pub struct GraphQLNormalizationPayload {
    pub operation_for_plan: OperationDefinition,
    pub operation_for_introspection: Option<OperationDefinition>,
    pub root_type_name: &'static str,
    pub projection_plan: Vec<FieldProjectionPlan>,
}

#[inline]
pub async fn normalize_op(
    req: &HttpRequest,
    execution_params: &ExecutionRequest,
    parser_payload: &GraphQLParserPayload,
    app_state: &GatewaySharedState,
) -> Result<Arc<GraphQLNormalizationPayload>, PipelineError> {
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
        Some(payload) => Ok(payload),
        None => match normalize_operation(
            &app_state.planner.supergraph,
            &parser_payload.parsed_operation,
            execution_params.operation_name.as_deref(),
        ) {
            Ok(doc) => {
                let operation = doc.operation;
                let (root_type_name, projection_plan) =
                    FieldProjectionPlan::from_operation(&operation, &app_state.schema_metadata);

                let partitioned_operation = partition_operation(operation);
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

                Ok(payload_arc)
            }
            Err(err) => {
                error!("Failed to normalize GraphQL operation: {}", err);
                trace!("{:?}", err);

                Err(req.new_pipeline_error(PipelineErrorVariant::NormalizationError(err)))
            }
        },
    }
}
