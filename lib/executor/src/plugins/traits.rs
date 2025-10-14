use std::{collections::HashMap, sync::Arc};

use hive_router_query_planner::consumer_schema::ConsumerSchema;
use hive_router_query_planner::planner::plan_nodes::QueryPlan;
use ntex::web::HttpRequest;
use ntex::web::HttpResponse;

use crate::response::graphql_error::GraphQLError;
use crate::response::value::Value;

pub enum ControlFlow<'a, TPayload> {
    Continue,
    Break(HttpResponse),
    OnEnd(Box<dyn FnOnce(TPayload) -> ControlFlow<'a, ()> + Send + 'a>),
}

pub struct ExecutionResult<'exec> {
    pub data: &'exec mut Value<'exec>,
    pub errors: &'exec mut Vec<GraphQLError>,
    pub extensions: &'exec mut Option<HashMap<String, Value<'exec>>>,
}

pub struct OnExecutePayload<'exec> {
    pub router_http_request: &'exec HttpRequest,
    pub query_plan: Arc<QueryPlan>,

    pub data: &'exec mut Value<'exec>,
    pub errors: &'exec mut Vec<GraphQLError>,
    pub extensions: &'exec mut HashMap<String, sonic_rs::Value>,

    pub skip_execution: bool,

    pub variable_values: &'exec Option<HashMap<String, sonic_rs::Value>>,
}

pub trait RouterPlugin {
    fn on_execute<'exec>(
        &self, 
        _payload: OnExecutePayload<'exec>,
    ) -> ControlFlow<'exec, OnExecutePayload<'exec>> {
        ControlFlow::Continue
    }
    fn on_schema_reload(&self, _payload: OnSchemaReloadPayload) {}
}

pub struct OnExecuteEndPayload<'exec> {
    pub router_http_request: &'exec HttpRequest,
    pub query_plan: Arc<QueryPlan>,

    pub data: &'exec Value<'exec>,
    pub errors: &'exec Vec<GraphQLError>,
    pub extensions: &'exec mut HashMap<String, sonic_rs::Value>,

    pub variable_values: &'exec Option<HashMap<String, sonic_rs::Value>>,
}

pub struct OnSchemaReloadPayload {
    pub old_schema: &'static ConsumerSchema,
    pub new_schema: &'static mut ConsumerSchema,
}
