use bytes::Bytes;

use crate::{
    executors::{
        common::SubgraphExecutionRequest,
        http::{DeduplicationHint, SubgraphHttpResponse},
    },
    plugin_context::PluginContext,
    plugin_trait::{
        from_graphql_error_to_bytes, EndHookPayload, FromGraphQLErrorToResponse, StartHookPayload,
    },
    response::graphql_error::GraphQLError,
};

pub struct OnSubgraphHttpRequestHookPayload<'exec> {
    /// The name of the subgraph for which the HTTP request is being sent.
    pub subgraph_name: &'exec str,

    /// The endpoint of the subgraph for which the HTTP request is being sent.
    pub endpoint: &'exec http::Uri,
    /// The HTTP method of the request being sent to the subgraph.
    pub method: http::Method,
    /// The raw body of the HTTP request being sent to the subgraph.
    pub body: Vec<u8>,
    /// The original GraphQL request that is being executed and for which the HTTP request is being sent to the subgraph.
    pub execution_request: SubgraphExecutionRequest<'exec>,

    /// The flag indicating whether the request should be deduplicated or not.
    /// If this is `true`, the router will check if there is an ongoing request with the same subgraph name and execution request,
    /// and if there is, it will wait for the ongoing request to finish and return the same response instead of sending a new request to the subgraph.
    ///
    /// [Learn more about request deduplication](https://the-guild.dev/graphql/hive/docs/router/guides/performance-tuning#request-deduplication)
    pub deduplicate_request: bool,

    /// The context object that can be used to share data across different plugin hooks for the same request.
    /// It is unique per request and is dropped after the response is sent.
    ///
    /// [Learn more about the context data sharing in the docs](https://graphql-hive.com/docs/router/extensibility/plugin_system#context-data-sharing)
    pub context: &'exec PluginContext,
}

impl<'exec> StartHookPayload<OnSubgraphHttpResponseHookPayload<'exec>, SubgraphHttpResponse>
    for OnSubgraphHttpRequestHookPayload<'exec>
{
}

pub type OnSubgraphHttpRequestHookResult<'exec> = crate::plugin_trait::StartHookResult<
    'exec,
    OnSubgraphHttpRequestHookPayload<'exec>,
    OnSubgraphHttpResponseHookPayload<'exec>,
    SubgraphHttpResponse,
>;

pub struct OnSubgraphHttpResponseHookPayload<'exec> {
    /// The context object that can be used to share data across different plugin hooks for the same request.
    /// It is unique per request and is dropped after the response is sent.
    ///
    /// [Learn more about the context data sharing in the docs](https://graphql-hive.com/docs/router/extensibility/plugin_system#context-data-sharing)
    pub context: &'exec PluginContext,
    /// The HTTP response received from the subgraph for the HTTP request sent by the router.
    /// Plugins can modify the response before it is sent back to the client.
    pub response: SubgraphHttpResponse,
    /// The flag indicating whether the request was deduplicated or not.
    /// - If this is `DeduplicationHint::Deduped`, it means the request was deduplicated and the response is the result of an ongoing request with the same subgraph name and execution request.
    /// - If this is `DeduplicationHint::NotDeduped`, it means the request was not deduplicated and the response is the result of a new request sent to the subgraph.
    pub deduplication_hint: DeduplicationHint,
}

impl<'exec> EndHookPayload<SubgraphHttpResponse> for OnSubgraphHttpResponseHookPayload<'exec> {}

pub type OnSubgraphHttpResponseHookResult<'exec> = crate::plugin_trait::EndHookResult<
    OnSubgraphHttpResponseHookPayload<'exec>,
    SubgraphHttpResponse,
>;

impl FromGraphQLErrorToResponse for SubgraphHttpResponse {
    fn from_graphql_error_to_response(error: GraphQLError, status: http::StatusCode) -> Self {
        let body_bytes = from_graphql_error_to_bytes(error);
        SubgraphHttpResponse {
            body: Bytes::from(body_bytes),
            status,
            ..Default::default()
        }
    }
}
