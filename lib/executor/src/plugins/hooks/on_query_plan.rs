use hive_router_query_planner::{ast::operation::OperationDefinition, graph::PlannerOverrideContext, planner::{Planner, plan_nodes::QueryPlan}, utils::cancellation::CancellationToken};

use crate::plugin_trait::{EndPayload, StartPayload};

pub struct OnQueryPlanStartPayload<'exec> {
    pub router_http_request: &'exec ntex::web::HttpRequest,
    pub filtered_operation_for_plan: &'exec OperationDefinition,
    pub planner_override_context: PlannerOverrideContext,
    pub cancellation_token: &'exec CancellationToken,
    pub query_plan: Option<QueryPlan>,
    pub planner: &'exec Planner,
}

impl<'exec> StartPayload<OnQueryPlanEndPayload<'exec>> for OnQueryPlanStartPayload<'exec> {}

pub struct OnQueryPlanEndPayload<'exec> {
    pub router_http_request: &'exec ntex::web::HttpRequest,
    pub filtered_operation_for_plan: &'exec OperationDefinition,
    pub planner_override_context: PlannerOverrideContext,
    pub cancellation_token: &'exec CancellationToken,
    pub query_plan: QueryPlan,
    pub planner: &'exec Planner,
}

impl<'exec> EndPayload for OnQueryPlanEndPayload<'exec> {}