use std::collections::HashMap;

use hive_router_query_planner::planner::plan_nodes::QueryPlan;
use ntex::web::HttpRequest;

use crate::plugin_trait::{EndPayload, StartPayload};
use crate::response::{value::Value};
use crate::response::graphql_error::GraphQLError;

pub struct OnExecuteStartPayload<'exec> {
    pub router_http_request: &'exec HttpRequest,
    pub query_plan: &'exec QueryPlan,

    pub data: &'exec mut Value<'exec>,
    pub errors: &'exec mut Vec<GraphQLError>,
    pub extensions: &'exec mut HashMap<String, sonic_rs::Value>,

    pub skip_execution: &'exec mut bool,

    pub variable_values: &'exec Option<HashMap<String, sonic_rs::Value>>,

    pub dedupe_subgraph_requests: &'exec mut bool,
}

impl<'exec> StartPayload<OnExecuteEndPayload<'exec>> for OnExecuteStartPayload<'exec> {}

pub struct OnExecuteEndPayload<'exec> {
    pub router_http_request: &'exec HttpRequest,
    pub query_plan: &'exec QueryPlan,


    pub data: &'exec mut Value<'exec>,
    pub errors: &'exec mut Vec<GraphQLError>,
    pub extensions: &'exec mut HashMap<String, sonic_rs::Value>,

    pub variable_values: &'exec Option<HashMap<String, sonic_rs::Value>>,

    pub dedupe_subgraph_requests: &'exec mut bool,
}

impl<'exec> EndPayload for OnExecuteEndPayload<'exec> {}
