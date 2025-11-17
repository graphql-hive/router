use std::hash::{Hash, Hasher};
use std::sync::Arc;

use hive_router_plan_executor::hooks::on_deserialization::GraphQLParams;
use hive_router_plan_executor::introspection::partition::partition_operation;
use hive_router_plan_executor::projection::plan::FieldProjectionPlan;
use hive_router_query_planner::ast::normalization::normalize_operation;
use hive_router_query_planner::ast::operation::OperationDefinition;
use ntex::web::HttpRequest;
use xxhash_rust::xxh3::Xxh3;

use crate::pipeline::error::{PipelineError, PipelineErrorFromAcceptHeader, PipelineErrorVariant};
use crate::pipeline::parser::GraphQLParserPayload;
use crate::schema_state::{SchemaState, SupergraphData};
use tracing::{error, trace};

#[derive(Debug)]
pub struct GraphQLNormalizationPayload {
    /// The operation to execute, without introspection fields.
    pub operation_for_plan: OperationDefinition,
    pub operation_for_introspection: Option<OperationDefinition>,
    pub root_type_name: &'static str,
    pub projection_plan: Vec<FieldProjectionPlan>,
}

#[inline]
pub async fn normalize_request_with_cache(
    req: &HttpRequest,
    supergraph: &SupergraphData,
    schema_state: &Arc<SchemaState>,
    graphql_params: &GraphQLParams,
    parser_payload: &GraphQLParserPayload,
) -> Result<Arc<GraphQLNormalizationPayload>, PipelineError> {
    let cache_key = match &graphql_params.operation_name {
        Some(operation_name) => {
            let mut hasher = Xxh3::new();
            graphql_params.query.hash(&mut hasher);
            operation_name.hash(&mut hasher);
            hasher.finish()
        }
        None => parser_payload.cache_key,
    };

    match schema_state.normalize_cache.get(&cache_key).await {
        Some(payload) => {
            trace!(
                "Found normalized GraphQL operation in cache (operation name={:?}): {}",
                payload.operation_for_plan.name,
                payload.operation_for_plan
            );

            Ok(payload)
        }
        None => match normalize_operation(
            &supergraph.planner.supergraph,
            &parser_payload.parsed_operation,
            graphql_params.operation_name.as_deref(),
        ) {
            Ok(doc) => {
                trace!(
                    "Successfully normalized GraphQL operation (operation name={:?}): {}",
                    doc.operation_name,
                    doc.operation
                );

                let operation = doc.operation;
                let (root_type_name, projection_plan) =
                    FieldProjectionPlan::from_operation(&operation, &supergraph.metadata);
                let partitioned_operation = partition_operation(operation);

                let payload = GraphQLNormalizationPayload {
                    root_type_name,
                    projection_plan,
                    operation_for_plan: partitioned_operation.downstream_operation,
                    operation_for_introspection: partitioned_operation.introspection_operation,
                };
                let payload_arc = Arc::new(payload);
                schema_state
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
