use hive_router_query_planner::{
    ast::operation::OperationDefinition,
    graph::PlannerOverrideContext,
    planner::{plan_nodes::QueryPlan, Planner},
    utils::cancellation::CancellationToken,
};

use crate::{
    execution::plan::PlanExecutionOutput,
    plugin_context::{PluginContext, RouterHttpRequest},
    plugin_trait::{EndHookPayload, EndHookResult, StartHookPayload, StartHookResult},
};

pub struct OnQueryPlanStartHookPayload<'exec> {
    pub router_http_request: &'exec RouterHttpRequest<'exec>,
    pub context: &'exec PluginContext,
    pub filtered_operation_for_plan: &'exec OperationDefinition,
    pub planner_override_context: PlannerOverrideContext,
    pub cancellation_token: &'exec CancellationToken,
    pub planner: &'exec Planner,
    // Override
    pub query_plan: Option<QueryPlan>,
}

impl<'exec> OnQueryPlanStartHookPayload<'exec> {
    pub fn with_query_plan(mut self, query_plan: QueryPlan) -> Self {
        self.query_plan = Some(query_plan);
        self
    }
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
    pub query_plan: QueryPlan,
}

impl EndHookPayload<PlanExecutionOutput> for OnQueryPlanEndHookPayload {}

pub type OnQueryPlanEndHookResult = EndHookResult<OnQueryPlanEndHookPayload, PlanExecutionOutput>;
