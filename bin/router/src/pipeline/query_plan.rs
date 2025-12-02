use std::hash::{Hash, Hasher};
use std::sync::Arc;

use crate::pipeline::error::PipelineErrorVariant;
use crate::pipeline::normalize::GraphQLNormalizationPayload;
use crate::pipeline::progressive_override::{RequestOverrideContext, StableOverrideContext};
use crate::schema_state::SchemaState;
use hive_router_plan_executor::executors::http::HttpResponse;
use hive_router_plan_executor::hooks::on_query_plan::{
    OnQueryPlanEndHookPayload, OnQueryPlanStartHookPayload,
};
use hive_router_plan_executor::hooks::on_supergraph_load::SupergraphData;
use hive_router_plan_executor::plugin_context::PluginRequestState;
use hive_router_plan_executor::plugin_trait::{EndControlFlow, StartControlFlow};
use hive_router_query_planner::planner::plan_nodes::QueryPlan;
use hive_router_query_planner::planner::PlannerError;
use hive_router_query_planner::utils::cancellation::CancellationToken;
use xxhash_rust::xxh3::Xxh3;

pub enum QueryPlanResult {
    QueryPlan(Arc<QueryPlan>),
    Response(HttpResponse),
}

pub enum QueryPlanGetterError {
    Planner(PlannerError),
    Response(HttpResponse),
}

#[inline]
pub async fn plan_operation_with_cache<'req>(
    supergraph: &SupergraphData,
    schema_state: &SchemaState,
    normalized_operation: &GraphQLNormalizationPayload,
    request_override_context: &RequestOverrideContext,
    cancellation_token: &CancellationToken,
    plugin_req_state: &Option<PluginRequestState<'req>>,
) -> Result<QueryPlanResult, PipelineErrorVariant> {
    let stable_override_context =
        StableOverrideContext::new(&supergraph.planner.supergraph, request_override_context);

    let filtered_operation_for_plan = &normalized_operation.operation_for_plan;
    let plan_cache_key =
        calculate_cache_key(filtered_operation_for_plan.hash(), &stable_override_context);
    let is_pure_introspection = filtered_operation_for_plan.selection_set.is_empty()
        && normalized_operation.operation_for_introspection.is_some();

    let plan_result = schema_state
        .plan_cache
        .try_get_with(plan_cache_key, async {
            if is_pure_introspection {
                return Ok(Arc::new(QueryPlan {
                    kind: "QueryPlan".to_string(),
                    node: None,
                }));
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
                        StartControlFlow::Continue => {
                            // continue to next plugin
                        }
                        StartControlFlow::EndResponse(response) => {
                            return Err(QueryPlanGetterError::Response(response));
                        }
                        StartControlFlow::OnEnd(callback) => {
                            on_end_callbacks.push(callback);
                        }
                    }
                }

                query_plan = start_payload.query_plan;
            }

            let query_plan = match query_plan {
                Some(plan) => plan,
                None => supergraph
                    .planner
                    .plan_from_normalized_operation(
                        filtered_operation_for_plan,
                        (&request_override_context.clone()).into(),
                        cancellation_token,
                    )
                    .map_err(QueryPlanGetterError::Planner)?,
            };

            let mut end_payload = OnQueryPlanEndHookPayload { query_plan };

            for callback in on_end_callbacks {
                let result = callback(end_payload);
                end_payload = result.payload;
                match result.control_flow {
                    EndControlFlow::Continue => {
                        // continue to next callback
                    }
                    EndControlFlow::EndResponse(response) => {
                        return Err(QueryPlanGetterError::Response(response));
                    }
                }
            }

            Ok(Arc::new(end_payload.query_plan))
            /* Handle on_query_plan hook in the plugins - END */
        })
        .await;

    match plan_result {
        Ok(plan) => Ok(QueryPlanResult::QueryPlan(plan)),
        Err(e) => match e.as_ref() {
            QueryPlanGetterError::Planner(e) => Err(PipelineErrorVariant::PlannerError(e.clone())),
            QueryPlanGetterError::Response(response) => {
                Ok(QueryPlanResult::Response(response.clone()))
            }
        },
    }
}

#[inline]
fn calculate_cache_key(operation_hash: u64, context: &StableOverrideContext) -> u64 {
    let mut hasher = Xxh3::new();
    operation_hash.hash(&mut hasher);
    context.hash(&mut hasher);
    hasher.finish()
}
