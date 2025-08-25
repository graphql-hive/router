use std::hash::{Hash, Hasher};
use std::sync::Arc;

use crate::pipeline::error::{PipelineError, PipelineErrorFromAcceptHeader, PipelineErrorVariant};
use crate::pipeline::normalize::GraphQLNormalizationPayload;
use crate::pipeline::progressive_override::{RequestOverrideContext, StableOverrideContext};
use crate::shared_state::GatewaySharedState;
use axum::body::Body;
use http::Request;
use query_planner::planner::plan_nodes::QueryPlan;
use tracing::debug;
use xxhash_rust::xxh3::Xxh3;

#[inline]
pub async fn plan_operation_with_cache(
    req: &mut Request<Body>,
    app_state: &Arc<GatewaySharedState>,
    normalized_operation: &Arc<GraphQLNormalizationPayload>,
    request_override_context: &RequestOverrideContext,
) -> Result<Arc<QueryPlan>, PipelineError> {
    let stable_override_context =
        StableOverrideContext::new(&app_state.planner.supergraph, request_override_context);

    let filtered_operation_for_plan = &normalized_operation.operation_for_plan;
    let plan_cache_key =
        calculate_cache_key(filtered_operation_for_plan.hash(), &stable_override_context);

    let query_plan_arc = match app_state.plan_cache.get(&plan_cache_key).await {
        Some(plan) => plan,
        None => {
            let plan = if filtered_operation_for_plan.selection_set.is_empty()
                && normalized_operation.operation_for_introspection.is_some()
            {
                debug!(
                    "No need for a plan, as the incoming query only involves introspection fields"
                );

                QueryPlan {
                    kind: "QueryPlan".to_string(),
                    node: None,
                }
            } else {
                app_state
                    .planner
                    .plan_from_normalized_operation(
                        filtered_operation_for_plan,
                        request_override_context.into(),
                    )
                    .map_err(|err| {
                        req.new_pipeline_error(PipelineErrorVariant::PlannerError(err))
                    })?
            };

            let arc_plan = Arc::new(plan);
            app_state
                .plan_cache
                .insert(plan_cache_key, arc_plan.clone())
                .await;

            arc_plan
        }
    };

    Ok(query_plan_arc)
}

fn calculate_cache_key(operation_hash: u64, context: &StableOverrideContext) -> u64 {
    let mut hasher = Xxh3::new();
    operation_hash.hash(&mut hasher);
    context.hash(&mut hasher);
    hasher.finish()
}
