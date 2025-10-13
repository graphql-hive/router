use std::{collections::HashMap, sync::Arc};

use hive_router_query_planner::consumer_schema::ConsumerSchema;
use hive_router_query_planner::planner::plan_nodes::QueryPlan;
use ntex::web::HttpRequest;
use ntex::web::HttpResponse;

use crate::response::graphql_error::GraphQLError;
use crate::response::value::Value;

pub enum ControlFlow {
    Continue,
    Break(HttpResponse),
}

pub struct ExecutionResult<'exec> {
    pub data: &'exec mut Value<'exec>,
    pub errors: &'exec mut Vec<GraphQLError>,
    pub extensions: &'exec mut Option<HashMap<String, Value<'exec>>>,
}

pub struct OnExecuteStartPayload<'exec> {
    pub router_http_request: &'exec HttpRequest,
    pub query_plan: Arc<QueryPlan>,

    pub data: &'exec mut Value<'exec>,
    pub errors: &'exec mut Vec<GraphQLError>,
    pub extensions: Option<&'exec mut sonic_rs::Value>,

    pub skip_execution: bool,

    pub variable_values: &'exec Option<HashMap<String, sonic_rs::Value>>,
}

pub trait OnExecuteStart {
    fn on_execute_start(&self, payload: OnExecuteStartPayload) -> ControlFlow;
}

pub struct OnExecuteEndPayload<'exec> {
    pub router_http_request: &'exec HttpRequest,
    pub query_plan: Arc<QueryPlan>,

    pub data: &'exec Value<'exec>,
    pub errors: &'exec Vec<GraphQLError>,
    pub extensions: &'exec mut HashMap<String, sonic_rs::Value>,

    pub variable_values: &'exec Option<HashMap<String, sonic_rs::Value>>,
}

pub trait OnExecuteEnd {
    fn on_execute_end(&self, payload: OnExecuteEndPayload) -> ControlFlow;
}

pub struct OnSchemaReloadPayload {
    pub old_schema: &'static ConsumerSchema,
    pub new_schema: &'static mut ConsumerSchema,
}

pub trait OnSchemaReload {
    fn on_schema_reload(&self, payload: OnSchemaReloadPayload);
}
