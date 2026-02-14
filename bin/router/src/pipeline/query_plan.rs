use std::hash::{Hash, Hasher};
use std::sync::{Arc, LazyLock};

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
use hive_router_plan_executor::plugin_trait::{EndControlFlow, StartControlFlow};
use hive_router_query_planner::ast::operation::OperationDefinition;
use hive_router_query_planner::planner::plan_nodes::QueryPlan;
use hive_router_query_planner::planner::query_plan::QUERY_PLAN_KIND;
use hive_router_query_planner::planner::PlannerError;
use hive_router_query_planner::utils::cancellation::CancellationToken;
use tracing::Instrument;
use xxhash_rust::xxh3::Xxh3;

pub enum QueryPlanResult {
    QueryPlan(Arc<QueryPlan>),
    EarlyResponse(PlanExecutionOutput),
}

pub enum QueryPlanError {
    Planner(PlannerError),
    EarlyResponse(PlanExecutionOutput),
}

static EMPTY_QUERY_PLAN: LazyLock<Arc<QueryPlan>> = LazyLock::new(|| {
    Arc::new(QueryPlan {
        kind: QUERY_PLAN_KIND,
        node: None,
    })
});

#[inline]
async fn get_query_plan(
    supergraph: &SupergraphData,
    normalized_operation: &GraphQLNormalizationPayload,
    filtered_operation_for_plan: &OperationDefinition,
    request_override_context: &RequestOverrideContext,
    cancellation_token: &CancellationToken,
    plugin_req_state: &Option<PluginRequestState<'_>>,
) -> Result<Arc<QueryPlan>, QueryPlanError> {
    let is_plan_operation_empty = filtered_operation_for_plan.selection_set.is_empty();
    let is_projection_plan_empty = normalized_operation.projection_plan.is_empty();
    let contains_introspection = normalized_operation.operation_for_introspection.is_some();
    let is_pure_introspection = is_plan_operation_empty && contains_introspection;

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

    let mut query_plan: Option<QueryPlan> = None;
    let mut on_end_callbacks = vec![];

    if let Some(plugin_req_state) = plugin_req_state {
        /* Handle on_query_plan hook in the plugins - START */
        let mut start_payload = OnQueryPlanStartHookPayload {
            router_http_request: &plugin_req_state.router_http_request,
            context: &plugin_req_state.context,
            filtered_operation_for_plan,
            planner_override_context: (&request_override_context.clone()).into(),
            cancellation_token,
            query_plan,
            planner: &supergraph.planner,
        };

        for plugin in plugin_req_state.plugins.as_ref() {
            let result = plugin.on_query_plan(start_payload).await;
            start_payload = result.payload;
            match result.control_flow {
                StartControlFlow::Proceed => {
                    // continue to next plugin
                }
                StartControlFlow::EndWithResponse(response) => {
                    return Err(QueryPlanError::EarlyResponse(response));
                }
                StartControlFlow::OnEnd(callback) => {
                    on_end_callbacks.push(callback);
                }
            }
        }

        // Give the ownership back to variables
        query_plan = start_payload.query_plan;
    }

    let mut query_plan = match query_plan {
        Some(plan) => plan,
        None => supergraph
            .planner
            .plan_from_normalized_operation(
                filtered_operation_for_plan,
                (&request_override_context.clone()).into(),
                cancellation_token,
            )
            .map_err(QueryPlanError::Planner)?,
    };

    if !on_end_callbacks.is_empty() {
        let mut end_payload = OnQueryPlanEndHookPayload { query_plan };

        for callback in on_end_callbacks {
            let result = callback(end_payload);
            end_payload = result.payload;
            match result.control_flow {
                EndControlFlow::Proceed => {
                    // continue to next callback
                }
                EndControlFlow::EndWithResponse(response) => {
                    return Err(QueryPlanError::EarlyResponse(response));
                }
            }
        }

        // Give the ownership back to variables
        query_plan = end_payload.query_plan;
    }

    Ok(query_plan.into())
}

#[inline]
pub async fn plan_operation_with_cache<'req>(
    supergraph: &SupergraphData,
    schema_state: &SchemaState,
    normalized_operation: &GraphQLNormalizationPayload,
    request_override_context: &RequestOverrideContext,
    cancellation_token: &CancellationToken,
    plugin_req_state: &Option<PluginRequestState<'req>>,
) -> Result<QueryPlanResult, PipelineError> {
    let plan_span = GraphQLPlanSpan::new();

    async {
        let stable_override_context =
            StableOverrideContext::new(&supergraph.planner.supergraph, request_override_context);

        let filtered_operation_for_plan = &normalized_operation.operation_for_plan;
        let plan_cache_key =
            calculate_cache_key(filtered_operation_for_plan.hash(), &stable_override_context);

        let plan = schema_state.plan_cache.get(&plan_cache_key).await;

        if let Some(cached_plan) = plan {
            tracing::trace!("query plan for this operation exists in the cache");
            plan_span.record_cache_hit(true);
            Ok(QueryPlanResult::QueryPlan(cached_plan))
        } else {
            plan_span.record_cache_hit(false);
            tracing::trace!("query plan for this operation does not exist in the cache");
            let result = get_query_plan(
                supergraph,
                normalized_operation,
                filtered_operation_for_plan,
                request_override_context,
                cancellation_token,
                plugin_req_state,
            )
            .await;

            match result {
                Ok(plan) => {
                    schema_state
                        .plan_cache
                        .insert(plan_cache_key, plan.clone())
                        .await;
                    Ok(QueryPlanResult::QueryPlan(plan))
                }
                Err(e) => match e {
                    QueryPlanError::Planner(e) => Err(PipelineError::PlannerError(e)),
                    QueryPlanError::EarlyResponse(response) => {
                        Ok(QueryPlanResult::EarlyResponse(response))
                    }
                },
            }
        }
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
