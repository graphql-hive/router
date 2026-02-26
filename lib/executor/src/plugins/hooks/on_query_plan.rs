use std::sync::Arc;

use hive_router_query_planner::{
    ast::operation::OperationDefinition,
    planner::{plan_nodes::QueryPlan, Planner},
    utils::cancellation::CancellationToken,
};

use crate::{
    execution::plan::PlanExecutionOutput,
    plugin_context::{PluginContext, RouterHttpRequest},
    plugin_trait::{CacheHint, EndHookPayload, EndHookResult, StartHookPayload, StartHookResult},
};

pub struct OnQueryPlanStartHookPayload<'exec> {
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
    /// The GraphQL Document AST that will be used for query planning.
    pub filtered_operation_for_plan: &'exec OperationDefinition,
    /// The cancellation token that can be used to check if the request has been cancelled by the client or not.
    pub cancellation_token: &'exec CancellationToken,
    /// The query planner instance that will be used to generate the query plan for the incoming GraphQL request.
    pub planner: &'exec Planner,
}

impl<'exec> StartHookPayload<OnQueryPlanEndHookPayload, PlanExecutionOutput>
    for OnQueryPlanStartHookPayload<'exec>
{
}

pub type OnQueryPlanStartHookResult<'exec> = StartHookResult<
    'exec,
    OnQueryPlanStartHookPayload<'exec>,
    OnQueryPlanEndHookPayload,
    PlanExecutionOutput,
>;

pub struct OnQueryPlanEndHookPayload {
    /// The generated query plan for the incoming GraphQL request.
    pub query_plan: Arc<QueryPlan>,
    /// The cache hint for the generated query plan.
    /// - If this is `CacheHint::Hit`, it means the query planning process didn't happen because the result was retrieved from the cache.
    /// - If this is `CacheHint::Miss`, it means the query planning process happened and the result was not retrieved from the cache.
    pub cache_hint: CacheHint,
}

impl EndHookPayload<PlanExecutionOutput> for OnQueryPlanEndHookPayload {}

pub type OnQueryPlanEndHookResult = EndHookResult<OnQueryPlanEndHookPayload, PlanExecutionOutput>;
