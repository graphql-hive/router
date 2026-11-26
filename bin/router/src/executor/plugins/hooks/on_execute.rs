use std::collections::HashMap;

use crate::query_planner::ast::operation::OperationDefinition;
use crate::query_planner::planner::plan_nodes::QueryPlan;
use serde::Serialize;
use sonic_rs::json;

use crate::executor::execution::plan::PlanExecutionOutput;
use crate::executor::headers::plan::HeaderAggregationStrategy;
use crate::executor::headers::response::ResponseHeaderAggregator;
use crate::executor::headers::sanitizer::is_denied_response_header;
use crate::executor::plugin_context::{PluginContext, RouterHttpRequest};
use crate::executor::plugin_trait::{
    from_graphql_errors_to_bytes, EarlyHTTPResponse, EndHookPayload, EndHookResult,
    FromGraphQLErrorToResponse, FromGraphQLErrorsToResponse, StartHookPayload, StartHookResult,
};
use crate::executor::request_context::RequestContextPluginApi;
use crate::executor::response::graphql_error::GraphQLError;
use crate::executor::response::value::Value;
use http::{HeaderName, HeaderValue};

type RequestContextApi = RequestContextPluginApi<super::OnExecute>;

/// Demand-control cost estimate available *before* execution.
///
/// Present on [`OnExecuteStartHookPayload`] when demand control is enabled.
/// The `actual` cost is not known at this stage — see [`DemandControlCost`].
#[derive(Debug, Clone, Copy)]
pub struct DemandControlEstimatedCost {
    /// The pre-execution estimated cost of the operation.
    pub estimated: u64,
    /// The configured supergraph-wide maximum cost.
    pub max: u64,
}

/// Demand-control cost available *after* execution.
///
/// Present on [`OnExecuteEndHookPayload`] when demand control is enabled.
#[derive(Debug, Clone, Copy)]
pub struct DemandControlCost {
    /// The pre-execution estimated cost of the operation.
    pub estimated: u64,
    /// The configured supergraph-wide maximum cost.
    pub max: u64,
    /// The post-execution actual cost, computed from the real response.
    pub actual: u64,
}

pub enum OnExecuteResponse {
    Output(PlanExecutionOutput),
    EarlyResponse(EarlyHTTPResponse),
}

impl From<PlanExecutionOutput> for OnExecuteResponse {
    fn from(response: PlanExecutionOutput) -> Self {
        Self::Output(response)
    }
}

impl From<EarlyHTTPResponse> for OnExecuteResponse {
    fn from(response: EarlyHTTPResponse) -> Self {
        Self::EarlyResponse(response)
    }
}

pub struct OnExecuteStartHookPayload<'exec> {
    /// The incoming HTTP request to the router for which the GraphQL execution is happening.
    /// It includes all the details of the request such as headers, body, etc.
    ///
    /// Example:
    /// ```
    ///  let my_header = payload.router_http_request.headers.get("my-header");
    ///  // do something with the header...
    ///  payload.proceed()
    /// ```
    pub router_http_request: &'exec RouterHttpRequest<'exec>,
    /// The context object that can be used to share data across different plugin hooks for the same request.
    /// It is unique per request and is dropped after the response is sent.
    ///
    /// [Learn more about the context data sharing in the docs](https://the-guild.dev/graphql/hive/docs/router/extensibility/plugin_system#context-data-sharing)
    pub context: &'exec PluginContext,
    pub request_context: RequestContextApi,
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

    /// The demand-control cost estimate for this operation, computed before execution.
    /// `None` when demand control is disabled. The `actual` cost is not available
    /// here — see [`OnExecuteEndHookPayload::demand_control_cost`].
    pub demand_control_estimate: Option<DemandControlEstimatedCost>,
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
    /// ```
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
    /// ```
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

impl<'exec> StartHookPayload<OnExecuteEndHookPayload<'exec>, OnExecuteResponse>
    for OnExecuteStartHookPayload<'exec>
{
}

pub type OnExecuteStartHookResult<'exec> = StartHookResult<
    'exec,
    OnExecuteStartHookPayload<'exec>,
    OnExecuteEndHookPayload<'exec>,
    OnExecuteResponse,
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
    pub request_context: RequestContextApi,

    /// An estimate of the response size in bytes.
    /// This is calculated based on the subgraph responses
    pub response_size_estimate: usize,

    /// The demand-control cost for this operation, including the post-execution
    /// `actual` cost. `None` when demand control is disabled.
    pub demand_control_cost: Option<DemandControlCost>,

    // aggregated client response headers (header rules, demand control, cache-control).
    // kept private so plugins only go through the accessor methods below. stored into the
    // shared response after all end callbacks so deduped joiners get the same headers
    pub(crate) response_headers: ResponseHeaderAggregator,
    /// `true` for an operation with `@defer` that sends more parts after this one.
    /// The hook then runs once, before the primary part, with the data fetched for it so far.
    /// What it changes in `data` shows in the later parts too.
    pub has_next: bool,
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

    /// Set a client response header, replacing any value already aggregated for it
    /// (for example from header rules or the computed `cache-control`).
    ///
    /// A `cache-control` set here cannot make a mutation or a response with GraphQL
    /// errors cacheable: in those cases the router replaces it with
    /// `no-store, no-cache, must-revalidate` after all end hooks run.
    ///
    /// Headers set here are part of the shared execution result, so with inbound
    /// request deduplication enabled, deduplicated requests receive them too even
    /// though they never run the `on_execute` hook themselves.
    ///
    /// Hop-by-hop headers and router-owned headers (like `content-type`) are ignored.
    ///
    /// Example:
    /// ```
    /// payload.insert_response_header(
    ///     HeaderName::from_static("cache-control"),
    ///     HeaderValue::from_static("no-store"),
    /// );
    /// ```
    pub fn insert_response_header(&mut self, name: HeaderName, value: HeaderValue) {
        if is_denied_response_header(&name) {
            return;
        }
        // drop existing values then go through write() so never-join headers
        // (like set-cookie) get the append strategy, same as header rules
        self.response_headers.entries.remove(&name);
        self.response_headers
            .write(&name, &value, HeaderAggregationStrategy::Last);
    }

    /// Append a value to a client response header, keeping the values already aggregated for it.
    /// Multiple values are joined with a comma, except for headers that must never be
    /// joined (like `set-cookie`), which are sent as separate header fields.
    ///
    /// Same as [`Self::insert_response_header`], the header is propagated to deduplicated
    /// requests, and hop-by-hop and router-owned headers are ignored.
    pub fn append_response_header(&mut self, name: HeaderName, value: HeaderValue) {
        if is_denied_response_header(&name) {
            return;
        }
        let (strategy, values) = self
            .response_headers
            .entries
            .entry(name)
            .or_insert_with(|| (HeaderAggregationStrategy::Append, vec![]));
        // force append, otherwise a first/last strategy from header rules would drop the value
        *strategy = HeaderAggregationStrategy::Append;
        values.push(value);
    }

    /// Get the values currently aggregated for a client response header.
    /// Returns `None` when the header is not set.
    pub fn get_response_header(&self, name: &HeaderName) -> Option<&[HeaderValue]> {
        self.response_headers
            .entries
            .get(name)
            .map(|(_, values)| values.as_slice())
    }

    /// Remove a client response header, returning the values that were aggregated for it.
    pub fn remove_response_header(&mut self, name: &HeaderName) -> Option<Vec<HeaderValue>> {
        self.response_headers
            .entries
            .remove(name)
            .map(|(_, values)| values)
    }
}

impl<'exec> EndHookPayload<OnExecuteResponse> for OnExecuteEndHookPayload<'exec> {}

pub type OnExecuteEndHookResult<'exec> =
    EndHookResult<OnExecuteEndHookPayload<'exec>, OnExecuteResponse>;

impl FromGraphQLErrorToResponse for OnExecuteResponse {
    fn from_graphql_error_to_response(error: GraphQLError, status_code: http::StatusCode) -> Self {
        Self::Output(PlanExecutionOutput::from_graphql_error_to_response(
            error,
            status_code,
        ))
    }
}

impl FromGraphQLErrorsToResponse for OnExecuteResponse {
    fn from_graphql_errors_to_response(
        errors: Vec<GraphQLError>,
        status_code: http::StatusCode,
    ) -> Self {
        Self::Output(PlanExecutionOutput::from_graphql_errors_to_response(
            errors,
            status_code,
        ))
    }
}

impl FromGraphQLErrorToResponse for PlanExecutionOutput {
    fn from_graphql_error_to_response(error: GraphQLError, status_code: http::StatusCode) -> Self {
        Self::from_graphql_errors_to_response(vec![error], status_code)
    }
}

impl FromGraphQLErrorsToResponse for PlanExecutionOutput {
    fn from_graphql_errors_to_response(
        errors: Vec<GraphQLError>,
        status_code: http::StatusCode,
    ) -> Self {
        let error_count = errors.len();
        PlanExecutionOutput {
            body: from_graphql_errors_to_bytes(errors),
            error_count,
            status_code,
        }
    }
}
