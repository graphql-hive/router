use std::sync::Arc;

use hive_router_query_planner::{
    ast::operation::OperationDefinition,
    planner::{plan_nodes::QueryPlan, Planner},
    utils::cancellation::CancellationToken,
};

use crate::{
    execution::plan::PlanExecutionOutput,
    plugin_context::{PluginContext, RouterHttpRequest},
    plugin_trait::{CacheHint, EndHookPayload, EndHookResult, StartHookPayload, StartHookResult},
};

pub struct OnQueryPlanStartHookPayload<'exec> {
    pub router_http_request: &'exec RouterHttpRequest<'exec>,
    pub context: &'exec PluginContext,
    pub filtered_operation_for_plan: &'exec OperationDefinition,
    pub cancellation_token: &'exec CancellationToken,
    pub planner: &'exec Planner,
}

impl<'exec> StartHookPayload<OnQueryPlanEndHookPayload, PlanExecutionOutput>
    for OnQueryPlanStartHookPayload<'exec>
{
}

pub type OnQueryPlanStartHookResult<'exec> = StartHookResult<
    'exec,
    OnQueryPlanStartHookPayload<'exec>,
    OnQueryPlanEndHookPayload,
    PlanExecutionOutput,
>;

pub struct OnQueryPlanEndHookPayload {
    pub query_plan: Arc<QueryPlan>,
    pub cache_hint: CacheHint,
}

impl EndHookPayload<PlanExecutionOutput> for OnQueryPlanEndHookPayload {}

pub type OnQueryPlanEndHookResult = EndHookResult<OnQueryPlanEndHookPayload, PlanExecutionOutput>;
