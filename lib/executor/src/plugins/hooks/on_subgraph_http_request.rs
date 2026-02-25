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
    pub subgraph_name: &'exec str,

    pub endpoint: &'exec http::Uri,
    pub method: http::Method,
    pub body: Vec<u8>,
    pub execution_request: SubgraphExecutionRequest<'exec>,

    pub deduplicate_request: bool,

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
    pub context: &'exec PluginContext,
    pub response: SubgraphHttpResponse,
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
