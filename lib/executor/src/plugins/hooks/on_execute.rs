use std::collections::HashMap;

use hive_router_query_planner::ast::operation::OperationDefinition;
use hive_router_query_planner::planner::plan_nodes::QueryPlan;
use serde::Serialize;
use sonic_rs::json;

use crate::execution::plan::PlanExecutionOutput;
use crate::plugin_context::{PluginContext, RouterHttpRequest};
use crate::plugin_trait::{
    EndHookPayload, EndHookResult, FromGraphQLErrorToResponse, StartHookPayload, StartHookResult,
};
use crate::response::graphql_error::GraphQLError;
use crate::response::value::Value;

pub struct OnExecuteStartHookPayload<'exec> {
    pub router_http_request: &'exec RouterHttpRequest<'exec>,
    pub context: &'exec PluginContext,
    pub query_plan: &'exec QueryPlan,
    pub operation_for_plan: &'exec OperationDefinition,

    pub data: Value<'exec>,
    pub errors: Vec<GraphQLError>,
    pub extensions: HashMap<String, sonic_rs::Value>,

    pub variable_values: &'exec Option<HashMap<String, sonic_rs::Value>>,

    pub dedupe_subgraph_requests: bool,
}

impl<'exec> OnExecuteStartHookPayload<'exec> {
    pub fn add_error(&mut self, error: GraphQLError) {
        self.errors.push(error);
    }
    pub fn filter_errors<F>(&mut self, mut f: F)
    where
        F: FnMut(&GraphQLError) -> bool,
    {
        self.errors.retain(|error| f(error))
    }
    pub fn add_extension<T: Serialize>(&mut self, key: &str, value: T) -> Option<sonic_rs::Value> {
        self.extensions.insert(key.into(), json!(value))
    }
    pub fn get_extension(&self, key: &str) -> Option<&sonic_rs::Value> {
        self.extensions.get(key)
    }
    pub fn remove_extension(&mut self, key: &str) -> Option<sonic_rs::Value> {
        self.extensions.remove(key)
    }
}

impl<'exec> StartHookPayload<OnExecuteEndHookPayload<'exec>, PlanExecutionOutput>
    for OnExecuteStartHookPayload<'exec>
{
}

pub type OnExecuteStartHookResult<'exec> = StartHookResult<
    'exec,
    OnExecuteStartHookPayload<'exec>,
    OnExecuteEndHookPayload<'exec>,
    PlanExecutionOutput,
>;

pub struct OnExecuteEndHookPayload<'exec> {
    pub data: Value<'exec>,
    pub errors: Vec<GraphQLError>,
    pub extensions: HashMap<String, sonic_rs::Value>,

    pub response_size_estimate: usize,
}

impl<'exec> OnExecuteEndHookPayload<'exec> {
    pub fn with_error(&mut self, error: GraphQLError) {
        self.errors.push(error);
    }
    pub fn filter_errors<F>(&mut self, mut f: F)
    where
        F: FnMut(&GraphQLError) -> bool,
    {
        self.errors.retain(|error| f(error))
    }
    pub fn add_extension<T: Serialize>(&mut self, key: &str, value: T) -> Option<sonic_rs::Value> {
        self.extensions.insert(key.into(), json!(value))
    }
    pub fn get_extension(&self, key: &str) -> Option<&sonic_rs::Value> {
        self.extensions.get(key)
    }
    pub fn remove_extension(&mut self, key: &str) -> Option<sonic_rs::Value> {
        self.extensions.remove(key)
    }
}

impl<'exec> EndHookPayload<PlanExecutionOutput> for OnExecuteEndHookPayload<'exec> {}

pub type OnExecuteEndHookResult<'exec> =
    EndHookResult<OnExecuteEndHookPayload<'exec>, PlanExecutionOutput>;

impl FromGraphQLErrorToResponse for PlanExecutionOutput {
    fn from_graphql_error_to_response(error: GraphQLError, status_code: http::StatusCode) -> Self {
        let body_json = json!({
            "errors": [error],
        });
        PlanExecutionOutput {
            body: sonic_rs::to_vec(&body_json).unwrap_or_default(),
            error_count: 1,
            response_headers_aggregator: None,
            status_code,
        }
    }
}
