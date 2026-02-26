use crate::{
    executors::common::{SubgraphExecutionRequest, SubgraphExecutorBoxedArc},
    plugin_context::{PluginContext, RouterHttpRequest},
    plugin_trait::{
        EndHookPayload, EndHookResult, FromGraphQLErrorToResponse, StartHookPayload,
        StartHookResult,
    },
    response::{graphql_error::GraphQLError, subgraph_response::SubgraphResponse},
};

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
    /// [Learn more about the context data sharing in the docs](https://graphql-hive.com/docs/router/extensibility/plugin_system#context-data-sharing)
    pub context: &'exec PluginContext,

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
    /// [Learn more about the context data sharing in the docs](https://graphql-hive.com/docs/router/extensibility/plugin_system#context-data-sharing)
    pub context: &'exec PluginContext,
}

impl<'exec> EndHookPayload<SubgraphResponse<'exec>> for OnSubgraphExecuteEndHookPayload<'exec> {}

pub type OnSubgraphExecuteEndHookResult<'exec> =
    EndHookResult<OnSubgraphExecuteEndHookPayload<'exec>, SubgraphResponse<'exec>>;

impl FromGraphQLErrorToResponse for SubgraphResponse<'_> {
    fn from_graphql_error_to_response(error: GraphQLError, _status_code: http::StatusCode) -> Self {
        SubgraphResponse {
            errors: Some(vec![error]),
            ..Default::default()
        }
    }
}
