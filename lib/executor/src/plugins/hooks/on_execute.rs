use std::collections::HashMap;

use hive_router_query_planner::ast::operation::OperationDefinition;
use hive_router_query_planner::planner::plan_nodes::QueryPlan;

use crate::plugin_context::{PluginContext, RouterHttpRequest};
use crate::plugin_trait::{EndHookPayload, EndHookResult, StartHookPayload, StartHookResult};
use crate::response::graphql_error::GraphQLError;
use crate::response::value::Value;

pub struct OnExecuteStartHookPayload<'exec> {
    pub router_http_request: &'exec RouterHttpRequest<'exec>,
    pub context: &'exec PluginContext,
    pub query_plan: &'exec QueryPlan,
    pub operation_for_plan: &'exec OperationDefinition,

    pub data: Value<'exec>,
    pub errors: Vec<GraphQLError>,
    pub extensions: Option<HashMap<String, sonic_rs::Value>>,

    pub variable_values: &'exec Option<HashMap<String, sonic_rs::Value>>,

    pub dedupe_subgraph_requests: bool,
}

impl<'exec> StartHookPayload<OnExecuteEndHookPayload<'exec>> for OnExecuteStartHookPayload<'exec> {}

pub type OnExecuteStartHookResult<'exec> =
    StartHookResult<'exec, OnExecuteStartHookPayload<'exec>, OnExecuteEndHookPayload<'exec>>;

pub struct OnExecuteEndHookPayload<'exec> {
    pub data: Value<'exec>,
    pub errors: Vec<GraphQLError>,
    pub extensions: Option<HashMap<String, sonic_rs::Value>>,

    pub response_size_estimate: usize,
}

impl<'exec> EndHookPayload for OnExecuteEndHookPayload<'exec> {}

pub type OnExecuteEndHookResult<'exec> = EndHookResult<OnExecuteEndHookPayload<'exec>>;
