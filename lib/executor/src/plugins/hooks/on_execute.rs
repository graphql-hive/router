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
    /// The incoming HTTP request to the router for which the GraphQL execution is happening.
    /// It includes all the details of the request such as headers, body, etc.
    ///
    /// Example:
    /// ```rust
    /// use hive_router::{
    ///     plugins::hooks::on_execute::{OnExecuteStartHookPayload, OnExecuteStartHookResult},
    /// };
    /// fn on_execute<'exec>(&'exec self, mut payload: OnExecuteStartHookPayload<'exec>) -> OnExecuteStartHookResult<'exec> {
    ///     let my_header = payload.router_http_request.headers.get("my-header");
    ///     // do something with the header...
    ///     payload.proceed()
    /// }
    /// ```
    pub router_http_request: &'exec RouterHttpRequest<'exec>,
    /// The context object that can be used to share data across different plugin hooks for the same request.
    /// It is unique per request and is dropped after the response is sent.
    ///
    /// [Learn more about the context data sharing in the docs](https://graphql-hive.com/docs/router/extensibility/plugin_system#context-data-sharing)
    pub context: &'exec PluginContext,
    /// The query plan generated for the incoming GraphQL request.
    /// It includes the details of how the router plans to execute the request across the subgraphs.
    pub query_plan: &'exec QueryPlan,
    /// The operation definition from the GraphQL document that is being executed.
    /// It includes the details of the operation such as its name, type (query/mutation/subscription), etc.
    pub operation_for_plan: &'exec OperationDefinition,

    /// The root value of the execution
    /// Anything here will be merged into the execution result
    pub data: Value<'exec>,
    /// Initial set of GraphQL errors in the execution result
    /// Any error passed here will be merged into the execution result errors list
    pub errors: Vec<GraphQLError>,
    /// Initial set of GraphQL extensions in the execution result
    /// Any extension passed here will be merged into the execution result extensions map
    pub extensions: HashMap<String, sonic_rs::Value>,

    /// Coerced variable values for the execution
    /// This includes all the variables from the request that have been coerced according to the variable definitions in the GraphQL document.
    /// [Learn more about coercion](https://graphql.org/learn/execution/#scalar-coercion)
    pub variable_values: &'exec Option<HashMap<String, sonic_rs::Value>>,

    pub dedupe_subgraph_requests: bool,
}

impl<'exec> OnExecuteStartHookPayload<'exec> {
    /// Add a GraphQL error to the execution result. This error will be merged into the execution result errors list.
    pub fn add_error(&mut self, error: GraphQLError) {
        self.errors.push(error);
    }
    /// Filter the GraphQL errors in the execution result.
    /// The provided closure should return `true` for the errors that should be kept,
    /// and `false` for the errors that should be removed.
    ///
    /// Example:
    /// ```rust
    /// use hive_router::{
    ///     plugins::hooks::on_execute::{OnExecuteStartHookPayload, OnExecuteStartHookResult},
    /// };
    /// fn on_execute<'exec>(&'exec self, mut payload: OnExecuteStartHookPayload<'exec>) -> OnExecuteStartHookResult<'exec> {
    ///    // Remove all errors with the message "Internal error"
    ///    payload.filter_errors(|error| error.message != "Internal error");
    ///    payload.proceed()
    /// }
    /// ```
    pub fn filter_errors<F>(&mut self, mut f: F)
    where
        F: FnMut(&GraphQLError) -> bool,
    {
        self.errors.retain(|error| f(error))
    }
    /// Add a GraphQL extension to the execution result. This extension will be merged into the execution result extensions map.
    /// Example:
    /// ```rust
    /// use hive_router::{
    ///     sonic_rs::json,
    ///     plugins::hooks::on_execute::{OnExecuteStartHookPayload, OnExecuteStartHookResult},
    /// };
    ///
    /// fn on_execute<'exec>(&'exec self, mut payload: OnExecuteStartHookPayload<'exec>) -> OnExecuteStartHookResult<'exec> {
    ///   // Add an extension with the key "my_extension" and value {"foo": "bar"}
    ///   payload.add_extension("my_extension", json!({"foo": "bar"}));
    ///   payload.proceed()
    /// }
    /// ```
    ///
    /// Then the result sent to the client will include this extension:
    /// ```json
    /// {
    ///   "data": { ... },
    ///   "errors": [ ... ],
    ///   "extensions": {
    ///     "my_extension": {
    ///       "foo": "bar"
    ///     }
    ///   }
    /// }
    /// ```
    pub fn add_extension<T: Serialize>(&mut self, key: &str, value: T) -> Option<sonic_rs::Value> {
        self.extensions.insert(key.into(), json!(value))
    }
    /// Get a reference to a GraphQL extension value from the execution result extensions map by its key.
    pub fn get_extension(&self, key: &str) -> Option<&sonic_rs::Value> {
        self.extensions.get(key)
    }
    /// Remove a GraphQL extension from the execution result extensions map by its key.
    /// This will remove the extension from the execution result.
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
    /// The final value of the execution result. This will be sent to the client as the "data" field in the GraphQL response.
    /// Plugins can modify this value before proceeding, and the modified value will be sent to the client.
    pub data: Value<'exec>,
    /// The final list of GraphQL errors in the execution result.
    /// This will be sent to the client as the "errors" field in the GraphQL response.
    /// Plugins can modify this list before proceeding, and the modified list will be sent to the client.
    pub errors: Vec<GraphQLError>,
    /// The final map of GraphQL extensions in the execution result.
    /// This will be sent to the client as the "extensions" field in the GraphQL response.
    /// Plugins can modify this map before proceeding, and the modified map will be sent to the client.
    pub extensions: HashMap<String, sonic_rs::Value>,

    /// An estimate of the response size in bytes.
    /// This is calculated based on the subgraph responses
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
