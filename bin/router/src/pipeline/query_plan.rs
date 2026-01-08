use std::hash::{Hash, Hasher};
use std::sync::{Arc, LazyLock};

use crate::cache_state::{CacheHitMiss, EntryResultHitMissExt};
use crate::pipeline::error::PipelineError;
use crate::pipeline::normalize::GraphQLNormalizationPayload;
use crate::pipeline::progressive_override::{RequestOverrideContext, StableOverrideContext};
use crate::schema_state::SchemaState;
use hive_router_internal::telemetry::traces::spans::graphql::GraphQLPlanSpan;
use hive_router_plan_executor::execution::plan::PlanExecutionOutput;
use hive_router_plan_executor::hooks::on_query_plan::{
    OnQueryPlanEndHookPayload, OnQueryPlanStartHookPayload,
};
use hive_router_plan_executor::hooks::on_supergraph_load::SupergraphData;
use hive_router_plan_executor::plugin_context::PluginRequestState;
use hive_router_plan_executor::plugin_trait::{CacheHint, EndControlFlow, StartControlFlow};
use hive_router_query_planner::planner::plan_nodes::QueryPlan;
use hive_router_query_planner::planner::query_plan::QUERY_PLAN_KIND;
use hive_router_query_planner::utils::cancellation::CancellationToken;
use tracing::Instrument;
use xxhash_rust::xxh3::Xxh3;

pub enum QueryPlanResult {
    QueryPlan(Arc<QueryPlan>),
    EarlyResponse(PlanExecutionOutput),
}
static EMPTY_QUERY_PLAN: LazyLock<Arc<QueryPlan>> = LazyLock::new(|| {
    Arc::new(QueryPlan {
        kind: QUERY_PLAN_KIND,
        node: None,
    })
});

#[inline]
pub async fn plan_operation_with_cache(
    supergraph: &SupergraphData,
    schema_state: &SchemaState,
    normalized_operation: &GraphQLNormalizationPayload,
    request_override_context: &RequestOverrideContext,
    cancellation_token: &CancellationToken,
    plugin_req_state: &Option<PluginRequestState<'_>>,
) -> Result<QueryPlanResult, PipelineError> {
    let plan_span = GraphQLPlanSpan::new();

    async {
        let mut on_end_callbacks = vec![];
        let filtered_operation_for_plan = &normalized_operation.operation_for_plan;
        if let Some(plugin_req_state) = plugin_req_state {
            let mut start_payload = OnQueryPlanStartHookPayload {
                router_http_request: &plugin_req_state.router_http_request,
                context: &plugin_req_state.context,
                filtered_operation_for_plan: &normalized_operation.operation_for_plan,
                cancellation_token,
                planner: &supergraph.planner,
            };

            for plugin in plugin_req_state.plugins.iter() {
                let result = plugin.on_query_plan(start_payload).await;
                start_payload = result.payload;
                match result.control_flow {
                    StartControlFlow::Proceed => {
                        // continue to next plugin
                    }
                    StartControlFlow::EndWithResponse(response) => {
                        return Ok(QueryPlanResult::EarlyResponse(response));
                    }
                    StartControlFlow::OnEnd(callback) => {
                        on_end_callbacks.push(callback);
                    }
                }
            }
        };

        let metrics = &schema_state.telemetry_context.metrics;
        let plan_cache_capture = metrics.cache.plan.capture_request();

        let stable_override_context =
            StableOverrideContext::new(&supergraph.planner.supergraph, request_override_context);
        let plan_cache_key =
            calculate_cache_key(filtered_operation_for_plan.hash(), &stable_override_context);
        let is_plan_operation_empty = filtered_operation_for_plan.selection_set.is_empty();
        let is_projection_plan_empty = normalized_operation.projection_plan.is_empty();
        let contains_introspection = normalized_operation.operation_for_introspection.is_some();
        let is_pure_introspection = is_plan_operation_empty && contains_introspection;

        let mut cache_hint = CacheHint::Hit;
        plan_span.record_cache_hit(true);
        let mut plan = schema_state
            .plan_cache
            .entry(plan_cache_key)
            .or_try_insert_with(async {
                if is_pure_introspection {
                    return Ok(EMPTY_QUERY_PLAN.clone());
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
                    return Ok(EMPTY_QUERY_PLAN.clone());
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
            .await
            .map_err(PipelineError::from)
            .into_result_with_hit_miss(|hit_miss| match hit_miss {
                CacheHitMiss::Hit => {
                    cache_hint = CacheHint::Hit;
                    plan_span.record_cache_hit(true);
                    plan_cache_capture.finish_hit();
                }
                CacheHitMiss::Miss | CacheHitMiss::Error => {
                    cache_hint = CacheHint::Miss;
                    plan_span.record_cache_hit(false);
                    plan_cache_capture.finish_miss();
                }
            })?;

        if !on_end_callbacks.is_empty() {
            let mut end_payload = OnQueryPlanEndHookPayload {
                query_plan: plan,
                cache_hint,
            };
            for callback in on_end_callbacks {
                let result = callback(end_payload);
                end_payload = result.payload;
                match result.control_flow {
                    EndControlFlow::Proceed => {
                        // continue to next callback
                    }
                    EndControlFlow::EndWithResponse(response) => {
                        return Ok(QueryPlanResult::EarlyResponse(response));
                    }
                }
            }
            // Give the ownership back to variables
            plan = end_payload.query_plan;
        }

        Ok(QueryPlanResult::QueryPlan(plan))
    }
    .instrument(plan_span.clone())
    .await
}

#[inline]
fn calculate_cache_key(operation_hash: u64, context: &StableOverrideContext) -> u64 {
    let mut hasher = Xxh3::new();
    operation_hash.hash(&mut hasher);
    context.hash(&mut hasher);
    hasher.finish()
}
