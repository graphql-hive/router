use crate::{
    executors::common::{SubgraphExecutionRequest, SubgraphExecutorBoxedArc},
    plugin_context::{PluginContext, RouterHttpRequest},
    plugin_trait::{
        EndHookPayload, EndHookResult, FromGraphQLErrorToResponse, FromGraphQLErrorsToResponse,
        StartHookPayload, StartHookResult,
    },
    request_context::RequestContextPluginApi,
    response::{graphql_error::GraphQLError, subgraph_response::SubgraphResponse},
};

type RequestContextApi = RequestContextPluginApi<super::OnSubgraphExecute>;

/// # Subscribe-path control-flow contract
///
/// `on_subgraph_execute` fires for queries, mutations **and** the subscribe
/// registration request (since [#1072]). The subscribe path imposes two
/// restrictions on the returned [`StartHookResult`]:
///
/// * [`StartControlFlow::EndWithResponse`] short-circuits the subgraph call
///   with a `SubgraphResponse<'exec>` that does not yet have a defined
///   materialisation into the `'static` stream returned by `subscribe`.
///   Returning this variant on the subscribe path surfaces
///   `SUBGRAPH_SUBSCRIBE_PLUGIN_HOOK_UNSUPPORTED` to the caller. Short-circuit
///   from an earlier hook (`on_http_request`, `on_graphql_params`,
///   `on_query_plan`) instead.
/// * [`StartControlFlow::OnEnd`] callbacks are not invoked on the subscribe
///   path — there is no symmetric end-of-stream point yet. The plugin's
///   request-payload mutations made before returning `on_end` are still
///   applied. Plugins that need end-of-stream behaviour for subscriptions
///   should track it through a different hook.
///
/// Tracked in <https://github.com/graphql-hive/router/issues/922>.
///
/// [#1072]: https://github.com/graphql-hive/router/pull/1072
/// [`StartHookResult`]: crate::plugin_trait::StartHookResult
/// [`StartControlFlow::EndWithResponse`]: crate::plugin_trait::StartControlFlow::EndWithResponse
/// [`StartControlFlow::OnEnd`]: crate::plugin_trait::StartControlFlow::OnEnd
pub struct OnSubgraphExecuteStartHookPayload<'exec> {
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

    /// The name of the subgraph for which the execution is happening.
    pub subgraph_name: &'exec str,
    /// The executor instance that will be used to execute the query plan for the incoming GraphQL request.
    pub executor: SubgraphExecutorBoxedArc,

    /// The execution request object that contains all the details about the execution such as the query plan, variables, etc.
    pub execution_request: SubgraphExecutionRequest<'exec>,
}

impl<'exec> StartHookPayload<OnSubgraphExecuteEndHookPayload<'exec>, SubgraphResponse<'exec>>
    for OnSubgraphExecuteStartHookPayload<'exec>
{
}

pub type OnSubgraphExecuteStartHookResult<'exec> = StartHookResult<
    'exec,
    OnSubgraphExecuteStartHookPayload<'exec>,
    OnSubgraphExecuteEndHookPayload<'exec>,
    SubgraphResponse<'exec>,
>;

pub struct OnSubgraphExecuteEndHookPayload<'exec> {
    /// The execution result from the subgraph execution for the incoming GraphQL request.
    /// Plugins can modify the execution result before it is sent back to the client.
    pub execution_result: SubgraphResponse<'exec>,
    /// The context object that can be used to share data across different plugin hooks for the same request.
    /// It is unique per request and is dropped after the response is sent.
    ///
    /// [Learn more about the context data sharing in the docs](https://the-guild.dev/graphql/hive/docs/router/extensibility/plugin_system#context-data-sharing)
    pub context: &'exec PluginContext,
    pub request_context: RequestContextApi,
}

impl<'exec> EndHookPayload<SubgraphResponse<'exec>> for OnSubgraphExecuteEndHookPayload<'exec> {}

pub type OnSubgraphExecuteEndHookResult<'exec> =
    EndHookResult<OnSubgraphExecuteEndHookPayload<'exec>, SubgraphResponse<'exec>>;

impl FromGraphQLErrorToResponse for SubgraphResponse<'_> {
    fn from_graphql_error_to_response(error: GraphQLError, status_code: http::StatusCode) -> Self {
        Self::from_graphql_errors_to_response(vec![error], status_code)
    }
}

impl FromGraphQLErrorsToResponse for SubgraphResponse<'_> {
    fn from_graphql_errors_to_response(
        errors: Vec<GraphQLError>,
        _status_code: http::StatusCode,
    ) -> Self {
        SubgraphResponse {
            errors: Some(errors),
            ..Default::default()
        }
    }
}
