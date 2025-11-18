use std::collections::HashMap;

use hive_router_query_planner::planner::plan_nodes::QueryPlan;
use ntex::web::HttpRequest;

use crate::plugin_trait::{EndPayload, StartPayload};
use crate::response::{value::Value};
use crate::response::graphql_error::GraphQLError;

pub struct OnExecuteStartPayload<'exec> {
    pub router_http_request: &'exec HttpRequest,
    pub query_plan: &'exec QueryPlan,

    pub data: Value<'exec>,
    pub errors: Vec<GraphQLError>,
    pub extensions: Option<HashMap<String, sonic_rs::Value>>,

    pub variable_values: &'exec Option<HashMap<String, sonic_rs::Value>>,

    pub dedupe_subgraph_requests: bool,
}

impl<'exec> StartPayload<OnExecuteEndPayload<'exec>> for OnExecuteStartPayload<'exec> {}

pub struct OnExecuteEndPayload<'exec> {
    pub data: Value<'exec>,
    pub errors: Vec<GraphQLError>,
    pub extensions: Option<HashMap<String, sonic_rs::Value>>,

    pub response_size_estimate: usize,
}

impl<'exec> EndPayload for OnExecuteEndPayload<'exec> {}
