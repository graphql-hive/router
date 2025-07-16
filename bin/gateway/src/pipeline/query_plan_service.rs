use std::sync::Arc;

use crate::pipeline::error::{PipelineError, PipelineErrorVariant};
use crate::pipeline::gateway_layer::{
    GatewayPipelineLayer, GatewayPipelineStepDecision, ProcessorLayer,
};
use crate::pipeline::normalize_service::GraphQLNormalizationPayload;
use crate::shared_state::GatewaySharedState;
use axum::body::Body;
use http::Request;
use query_planner::planner::plan_nodes::QueryPlan;
use tracing::{debug, trace};

#[derive(Clone, Debug)]
pub struct QueryPlanPayload {
    pub query_plan: Arc<QueryPlan>,
}

#[derive(Clone, Debug, Default)]
pub struct QueryPlanService;

impl QueryPlanService {
    pub fn new_layer() -> ProcessorLayer<Self> {
        ProcessorLayer::new(Self)
    }
}

#[async_trait::async_trait]
impl GatewayPipelineLayer for QueryPlanService {
    #[tracing::instrument(level = "trace", name = "QueryPlanService", skip_all)]
    async fn process(
        &self,
        req: &mut Request<Body>,
    ) -> Result<GatewayPipelineStepDecision, PipelineError> {
        let normalized_operation = req
            .extensions()
            .get::<Arc<GraphQLNormalizationPayload>>()
            .ok_or_else(|| {
                PipelineErrorVariant::InternalServiceError("GraphQLNormalizationPayload is missing")
            })?;

        let app_state = req
            .extensions()
            .get::<Arc<GatewaySharedState>>()
            .ok_or_else(|| {
                PipelineErrorVariant::InternalServiceError("GatewaySharedState is missing")
            })?;

        let filtered_operation_for_plan = &normalized_operation.operation_for_plan;
        let plan_cache_key = filtered_operation_for_plan.hash();

        let query_plan_arc = match app_state.plan_cache.get(&plan_cache_key).await {
            Some(plan) => {
                trace!("Plan with hash key {} was found in cache", plan_cache_key);

                plan
            }
            None => {
                trace!(
                    "Plan with hash key {} was not found in cache, planning...",
                    plan_cache_key
                );

                let plan = if filtered_operation_for_plan.selection_set.is_empty()
                    && normalized_operation.has_introspection
                {
                    debug!("No need for a plan, as the incoming query only involves introspection fields");

                    QueryPlan {
                        kind: "QueryPlan".to_string(),
                        node: None,
                    }
                } else {
                    match app_state
                        .planner
                        .plan_from_normalized_operation(filtered_operation_for_plan)
                    {
                        Ok(p) => p,
                        Err(err) => return Err(PipelineErrorVariant::PlannerError(err).into()),
                    }
                };

                trace!(
                    "Plan with hash key {} built and stored in cache:\n{}",
                    plan_cache_key,
                    plan
                );

                trace!("complete plan object:\n{:?}", plan);

                let arc_plan = Arc::new(plan);
                app_state
                    .plan_cache
                    .insert(plan_cache_key, arc_plan.clone())
                    .await;

                arc_plan
            }
        };

        req.extensions_mut().insert(QueryPlanPayload {
            query_plan: query_plan_arc,
        });

        Ok(GatewayPipelineStepDecision::Continue)
    }
}
