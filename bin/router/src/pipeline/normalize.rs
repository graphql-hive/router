use std::hash::{Hash, Hasher};
use std::sync::Arc;

use hive_router_internal::telemetry::traces::spans::graphql::{
    GraphQLNormalizeSpan, GraphQLSpanOperationIdentity, RecordOperationIdentity,
};
use hive_router_plan_executor::introspection::partition::partition_operation;
use hive_router_plan_executor::projection::plan::FieldProjectionPlan;
use hive_router_query_planner::ast::normalization::normalize_operation;
use hive_router_query_planner::ast::operation::OperationDefinition;
use ntex::web::HttpRequest;
use xxhash_rust::xxh3::Xxh3;

use crate::pipeline::error::{PipelineError, PipelineErrorFromAcceptHeader, PipelineErrorVariant};
use crate::pipeline::execution_request::ExecutionRequest;
use crate::pipeline::parser::GraphQLParserPayload;
use crate::schema_state::{SchemaState, SupergraphData};
use tracing::{error, trace, Instrument};

#[derive(Debug, Clone)]
pub struct GraphQLNormalizationPayload {
    /// The operation to execute, without introspection fields.
    pub operation_for_plan: Arc<OperationDefinition>,
    pub operation_for_introspection: Option<Arc<OperationDefinition>>,
    pub root_type_name: &'static str,
    pub projection_plan: Arc<Vec<FieldProjectionPlan>>,
    pub operation_indentity: OperationIdentity,
}

#[derive(Debug, Clone)]
pub struct OperationIdentity {
    pub name: Option<String>,
    pub operation_type: String,
    /// Hash of the original document sent to the router, by the client.
    pub client_document_hash: String,
}

impl<'a> From<&'a OperationIdentity> for GraphQLSpanOperationIdentity<'a> {
    fn from(op_id: &'a OperationIdentity) -> Self {
        GraphQLSpanOperationIdentity {
            name: op_id.name.as_deref(),
            operation_type: &op_id.operation_type,
            client_document_hash: &op_id.client_document_hash,
        }
    }
}

#[inline]
pub async fn normalize_request_with_cache(
    req: &HttpRequest,
    supergraph: &SupergraphData,
    schema_state: &Arc<SchemaState>,
    execution_params: &ExecutionRequest,
    parser_payload: &GraphQLParserPayload,
) -> Result<Arc<GraphQLNormalizationPayload>, PipelineError> {
    let normalize_span = GraphQLNormalizeSpan::new();
    normalize_span.record_operation_identity(parser_payload.into());

    let cache_key = match &execution_params.operation_name {
        Some(operation_name) => {
            let mut hasher = Xxh3::new();
            execution_params.query.hash(&mut hasher);
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
            normalize_span.record_cache_hit(true);

            Ok(payload)
        }
        None => {
            normalize_span.record_cache_hit(false);
            match normalize_operation(
                &supergraph.planner.supergraph,
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
                        FieldProjectionPlan::from_operation(&operation, &supergraph.metadata);
                    let partitioned_operation = partition_operation(operation);

                    let payload = GraphQLNormalizationPayload {
                        root_type_name,
                        projection_plan: Arc::new(projection_plan),
                        operation_for_plan: Arc::new(partitioned_operation.downstream_operation),
                        operation_for_introspection: partitioned_operation
                            .introspection_operation
                            .map(Arc::new),
                        operation_indentity: OperationIdentity {
                            name: doc.operation_name.clone(),
                            operation_type: parser_payload.operation_type.clone(),
                            client_document_hash: parser_payload.cache_key_string.clone(),
                        },
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
            }
        }
    }
}
