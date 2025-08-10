use std::collections::BTreeMap;
use std::hash::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use crate::pipeline::error::{PipelineError, PipelineErrorFromAcceptHeader, PipelineErrorVariant};
use crate::pipeline::normalize_service::GraphQLNormalizationPayload;
use crate::pipeline::progressive_override_service::RequestOverrideContext;
use crate::shared_state::GatewaySharedState;
use ntex::web::HttpRequest;
use query_planner::planner::plan_nodes::QueryPlan;
use query_planner::state::supergraph_state::SupergraphState;
use tracing::{debug, trace};

/// Deterministic context representing the outcome of progressive override rules.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct StableOverrideContext {
    /// Stores the active status of only the flags relevant to the supergraph.
    active_flags: BTreeMap<String, bool>,
    /// Stores the boolean outcome of the percentage check for each relevant threshold.
    percentage_outcomes: BTreeMap<u64, bool>,
}

impl StableOverrideContext {
    fn new(
        supergraph: &SupergraphState,
        request_override_context: &RequestOverrideContext,
    ) -> Self {
        let mut active_flags = BTreeMap::new();
        for flag_name in &supergraph.progressive_overrides.flags {
            let is_active = request_override_context.active_flags.contains(flag_name);
            active_flags.insert(flag_name.clone(), is_active);
        }

        let mut percentage_outcomes = BTreeMap::new();
        for &threshold in &supergraph.progressive_overrides.percentages {
            let in_range = request_override_context.percentage_value < threshold;
            percentage_outcomes.insert(threshold, in_range);
        }

        StableOverrideContext {
            active_flags,
            percentage_outcomes,
        }
    }
}

#[derive(Clone, Debug)]
pub struct QueryPlanPayload {
    pub query_plan: Arc<QueryPlan>,
}

fn calculate_cache_key(operation_hash: u64, context: &StableOverrideContext) -> u64 {
    let mut hasher = DefaultHasher::new();
    operation_hash.hash(&mut hasher);
    context.hash(&mut hasher);
    hasher.finish()
}

#[inline]
pub async fn plan_query(
    req: &HttpRequest,
    app_state: &GatewaySharedState,
    request_override_context: &RequestOverrideContext,
    normalized_operation: &Arc<GraphQLNormalizationPayload>,
) -> Result<QueryPlanPayload, PipelineError> {
    let stable_override_context =
        StableOverrideContext::new(&app_state.planner.supergraph, request_override_context);

    let filtered_operation_for_plan = &normalized_operation.operation_for_plan;
    let plan_cache_key =
        calculate_cache_key(filtered_operation_for_plan.hash(), &stable_override_context);

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
                debug!(
                    "No need for a plan, as the incoming query only involves introspection fields"
                );

                QueryPlan {
                    kind: "QueryPlan".to_string(),
                    node: None,
                }
            } else {
                match app_state.planner.plan_from_normalized_operation(
                    filtered_operation_for_plan,
                    request_override_context.into(),
                ) {
                    Ok(p) => p,
                    Err(err) => {
                        return Err(req.new_pipeline_error(PipelineErrorVariant::PlannerError(err)))
                    }
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

    Ok(QueryPlanPayload {
        query_plan: query_plan_arc,
    })
}
