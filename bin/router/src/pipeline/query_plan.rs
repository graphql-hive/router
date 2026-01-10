use std::hash::{Hash, Hasher};
use std::sync::Arc;

use crate::pipeline::error::PipelineErrorVariant;
use crate::pipeline::normalize::GraphQLNormalizationPayload;
use crate::pipeline::progressive_override::{RequestOverrideContext, StableOverrideContext};
use crate::schema_state::{SchemaState, SupergraphData};
use hive_router_query_planner::planner::plan_nodes::QueryPlan;
use hive_router_query_planner::utils::cancellation::CancellationToken;
use xxhash_rust::xxh3::Xxh3;

#[inline]
pub async fn plan_operation_with_cache(
    supergraph: &SupergraphData,
    schema_state: &Arc<SchemaState>,
    normalized_operation: &GraphQLNormalizationPayload,
    request_override_context: &RequestOverrideContext,
    cancellation_token: &CancellationToken,
) -> Result<Arc<QueryPlan>, PipelineErrorVariant> {
    let stable_override_context =
        StableOverrideContext::new(&supergraph.planner.supergraph, request_override_context);

    let filtered_operation_for_plan = &normalized_operation.operation_for_plan;
    let plan_cache_key =
        calculate_cache_key(filtered_operation_for_plan.hash(), &stable_override_context);
    let is_plan_operation_empty = filtered_operation_for_plan.selection_set.is_empty();
    let is_projection_plan_empty = normalized_operation.projection_plan.is_empty();
    let contains_introspection = normalized_operation.operation_for_introspection.is_some();
    let is_pure_introspection = is_plan_operation_empty && contains_introspection;

    let plan_result = schema_state
        .plan_cache
        .try_get_with(plan_cache_key, async move {
            if is_pure_introspection {
                return Ok(Arc::new(QueryPlan {
                    kind: "QueryPlan".to_string(),
                    node: None,
                }));
            }

            // If the operation is empty, but the projection plan is not,,
            // we don't need to run the planner,
            // as there is nothing to plan,
            // but we can't error out either,
            // as it would unwind into PipelineError,
            // and the response would be malformed.
            //
            // One example here is a scenario when all requested fields
            // were unauthorized and stripped out from the operation,
            // but we still need to project nulls for them in the response.
            // That's why we return an empty plan,
            // and allow for response projection to happen later.
            if is_plan_operation_empty && !is_projection_plan_empty {
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
        Err(e) => Err(PipelineErrorVariant::PlannerError(e.clone())),
    }
}

#[inline]
fn calculate_cache_key(operation_hash: u64, context: &StableOverrideContext) -> u64 {
    let mut hasher = Xxh3::new();
    operation_hash.hash(&mut hasher);
    context.hash(&mut hasher);
    hasher.finish()
}
