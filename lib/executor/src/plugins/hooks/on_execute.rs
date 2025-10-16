use std::collections::HashMap;
use std::sync::Arc;

use hive_router_query_planner::planner::plan_nodes::QueryPlan;
use ntex::web::HttpRequest;

use crate::response::{value::Value};
use crate::response::graphql_error::GraphQLError;

pub struct OnExecutePayload<'exec> {
    pub router_http_request: &'exec HttpRequest,
    pub query_plan: Arc<QueryPlan>,

    pub data: &'exec mut Value<'exec>,
    pub errors: &'exec mut Vec<GraphQLError>,
    pub extensions: &'exec mut HashMap<String, sonic_rs::Value>,

    pub skip_execution: bool,

    pub variable_values: &'exec Option<HashMap<String, sonic_rs::Value>>,
}

pub struct OnExecuteEndPayload<'exec> {
    pub router_http_request: &'exec HttpRequest,
    pub query_plan: Arc<QueryPlan>,

    pub data: &'exec Value<'exec>,
    pub errors: &'exec Vec<GraphQLError>,
    pub extensions: &'exec mut HashMap<String, sonic_rs::Value>,

    pub variable_values: &'exec Option<HashMap<String, sonic_rs::Value>>,
}

