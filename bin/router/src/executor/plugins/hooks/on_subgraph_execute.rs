use std::sync::Arc;

use http::{HeaderMap, StatusCode};

use crate::executor::{
    executors::common::{SubgraphExecutionRequest, SubgraphExecutorBoxedArc},
    plugin_context::{PluginContext, PluginRequestState, RouterHttpRequest},
    plugin_trait::{
        EndControlFlow, EndHookPayload, EndHookResult, FromGraphQLErrorToResponse,
        FromGraphQLErrorsToResponse, StartHookPayload, StartHookResult,
    },
    plugins::response_data::SubgraphData,
    request_context::RequestContextPluginApi,
    response::{graphql_error::GraphQLError, subgraph_response::SubgraphResponse},
};

type RequestContextApi = RequestContextPluginApi<super::OnSubgraphExecute>;

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
    /// What the subgraph sent for this fetch, before it's merged into the response. Plugins walk
    /// it value by value and change it in ways that keep it matching the fetch's query, like
    /// `OnExecuteEndHookPayload::data`.
    ///
    /// Example:
    /// ```ignore
    /// payload.data.walk(|value| {
    ///     if value.is("User", "email") {
    ///         value.set_null_with_error("email is private");
    ///     }
    /// });
    /// ```
    pub data: SubgraphData<'exec>,
    /// The subgraph's GraphQL errors. Plugins can change the list. These, and the errors
    /// plugins attach to nulls in `data`, count as the subgraph's: their paths are mapped to the
    /// client's, they carry the subgraph's name and error masking applies to them.
    pub errors: Vec<GraphQLError>,
    /// The subgraph's response headers, when it sent any
    pub headers: Option<Arc<HeaderMap>>,
    /// The subgraph's HTTP status, when there is one
    pub status: Option<StatusCode>,
    /// The context object that can be used to share data across different plugin hooks for the same request.
    /// It is unique per request and is dropped after the response is sent.
    ///
    /// [Learn more about the context data sharing in the docs](https://the-guild.dev/graphql/hive/docs/router/extensibility/plugin_system#context-data-sharing)
    pub context: &'exec PluginContext,
    pub request_context: RequestContextApi,
}

/// A plugin's end hook, run by the pipeline once it has the subgraph's data
pub(crate) type OnSubgraphExecuteEndCallback<'exec> = Box<
    dyn FnOnce(OnSubgraphExecuteEndHookPayload<'exec>) -> OnSubgraphExecuteEndHookResult<'exec>
        + Send
        + 'exec,
>;

/// What the end hooks left
pub(crate) struct SubgraphEndOutcome<'exec> {
    pub data: SubgraphData<'exec>,
    pub errors: Vec<GraphQLError>,
    /// The response a hook ended with: it's used instead, the later hooks don't run
    pub replaced: Option<SubgraphResponse<'exec>>,
}

/// Runs the end hooks on a fetch's data
pub(crate) fn run_end_hooks<'exec>(
    hooks: Vec<OnSubgraphExecuteEndCallback<'exec>>,
    plugin_req_state: &'exec PluginRequestState<'exec>,
    data: SubgraphData<'exec>,
    errors: Vec<GraphQLError>,
    headers: Option<Arc<HeaderMap>>,
    status: Option<StatusCode>,
) -> SubgraphEndOutcome<'exec> {
    let mut payload = OnSubgraphExecuteEndHookPayload {
        data,
        errors,
        headers,
        status,
        context: &plugin_req_state.context,
        request_context: plugin_req_state
            .request_context
            .for_plugin::<super::OnSubgraphExecute>(),
    };
    for hook in hooks {
        let result = hook(payload);
        payload = result.payload;
        if let EndControlFlow::EndWithResponse(response) = result.control_flow {
            return SubgraphEndOutcome {
                data: payload.data,
                errors: payload.errors,
                replaced: Some(response),
            };
        }
    }
    SubgraphEndOutcome {
        data: payload.data,
        errors: payload.errors,
        replaced: None,
    }
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
