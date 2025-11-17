use graphql_tools::static_graphql::query::Document;
use hive_router_query_planner::planner::{Planner, plan_nodes::QueryPlan};

use crate::plugin_trait::{EndPayload, StartPayload};

pub struct OnQueryPlanStartPayload<'exec> {
    pub router_http_request: &'exec ntex::web::HttpRequest,
    pub document: &'exec Document,
    // Other params
    pub query_plan: &'exec mut Option<QueryPlan>,
    pub planner: &'exec Planner,
}

impl<'exec> StartPayload<OnQueryPlanEndPayload<'exec>> for OnQueryPlanStartPayload<'exec> {}

pub struct OnQueryPlanEndPayload<'exec> {
    pub router_http_request: &'exec ntex::web::HttpRequest,
    pub document: &'exec Document,
    // Other params
    pub query_plan: &'exec mut QueryPlan,
}

impl<'exec> EndPayload for OnQueryPlanEndPayload<'exec> {}