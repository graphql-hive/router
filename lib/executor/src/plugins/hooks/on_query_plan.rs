use hive_router_query_planner::{
    ast::operation::OperationDefinition,
    graph::PlannerOverrideContext,
    planner::{plan_nodes::QueryPlan, Planner},
    utils::cancellation::CancellationToken,
};

use crate::{
    plugin_context::{PluginContext, RouterHttpRequest},
    plugin_trait::{EndHookPayload, EndHookResult, StartHookPayload, StartHookResult},
};

pub struct OnQueryPlanStartHookPayload<'exec> {
    pub router_http_request: &'exec RouterHttpRequest<'exec>,
    pub context: &'exec PluginContext,
    pub filtered_operation_for_plan: &'exec OperationDefinition,
    pub planner_override_context: PlannerOverrideContext,
    pub cancellation_token: &'exec CancellationToken,
    pub query_plan: Option<QueryPlan>,
    pub planner: &'exec Planner,
}

impl<'exec> StartHookPayload<OnQueryPlanEndHookPayload> for OnQueryPlanStartHookPayload<'exec> {}

pub type OnQueryPlanStartHookResult<'exec> =
    StartHookResult<'exec, OnQueryPlanStartHookPayload<'exec>, OnQueryPlanEndHookPayload>;

pub struct OnQueryPlanEndHookPayload {
    pub query_plan: QueryPlan,
}

impl EndHookPayload for OnQueryPlanEndHookPayload {}

pub type OnQueryPlanEndHookResult = EndHookResult<OnQueryPlanEndHookPayload>;
