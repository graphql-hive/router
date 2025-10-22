use std::hash::{Hash, Hasher};
use std::sync::Arc;

use crate::pipeline::error::{PipelineError, PipelineErrorFromAcceptHeader, PipelineErrorVariant};
use crate::pipeline::normalize::GraphQLNormalizationPayload;
use crate::pipeline::progressive_override::{RequestOverrideContext, StableOverrideContext};
use crate::schema_state::{SchemaState, SupergraphData};
use hive_router_query_planner::planner::plan_nodes::QueryPlan;
use hive_router_query_planner::utils::cancellation::CancellationToken;
use ntex::web::HttpRequest;
use xxhash_rust::xxh3::Xxh3;

#[inline]
pub async fn plan_operation_with_cache(
    req: &HttpRequest,
    supergraph: &SupergraphData,
    schema_state: &Arc<SchemaState>,
    normalized_operation: &Arc<GraphQLNormalizationPayload>,
    request_override_context: &RequestOverrideContext,
    cancellation_token: &CancellationToken,
) -> Result<Arc<QueryPlan>, PipelineError> {
    let stable_override_context =
        StableOverrideContext::new(&supergraph.planner.supergraph, request_override_context);

    let filtered_operation_for_plan = &normalized_operation.operation_for_plan;
    let plan_cache_key =
        calculate_cache_key(filtered_operation_for_plan.hash(), &stable_override_context);
    let is_pure_introspection = filtered_operation_for_plan.selection_set.is_empty()
        && normalized_operation.operation_for_introspection.is_some();

    let plan_result = schema_state
        .plan_cache
        .try_get_with(plan_cache_key, async move {
            if is_pure_introspection {
                return Ok(Arc::new(QueryPlan {
                    kind: "QueryPlan".to_string(),
                    node: None,
                }));
            }

            supergraph
                .planner
                .plan_from_normalized_operation(
                    filtered_operation_for_plan,
                    (&request_override_context.clone()).into(),
                    cancellation_token,
                )
                .map(Arc::new)
        })
        .await;

    match plan_result {
        Ok(plan) => Ok(plan),
        Err(e) => Err(req.new_pipeline_error(PipelineErrorVariant::PlannerError(e.clone()))),
    }
}

#[inline]
fn calculate_cache_key(operation_hash: u64, context: &StableOverrideContext) -> u64 {
    let mut hasher = Xxh3::new();
    operation_hash.hash(&mut hasher);
    context.hash(&mut hasher);
    hasher.finish()
}
