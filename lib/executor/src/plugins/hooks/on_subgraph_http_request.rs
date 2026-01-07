use crate::{
    executors::{common::SubgraphExecutionRequest, http::HttpResponse},
    plugin_context::PluginContext,
    plugin_trait::{EndHookPayload, StartHookPayload},
};

pub struct OnSubgraphHttpRequestHookPayload<'exec> {
    pub subgraph_name: &'exec str,

    pub endpoint: &'exec http::Uri,
    pub method: http::Method,
    pub body: Vec<u8>,
    pub execution_request: SubgraphExecutionRequest<'exec>,

    pub context: &'exec PluginContext,
}

impl<'exec> StartHookPayload<OnSubgraphHttpResponseHookPayload<'exec>, HttpResponse>
    for OnSubgraphHttpRequestHookPayload<'exec>
{
}

pub type OnSubgraphHttpRequestHookResult<'exec> = crate::plugin_trait::StartHookResult<
    'exec,
    OnSubgraphHttpRequestHookPayload<'exec>,
    OnSubgraphHttpResponseHookPayload<'exec>,
    HttpResponse,
>;

pub struct OnSubgraphHttpResponseHookPayload<'exec> {
    pub context: &'exec PluginContext,
    pub response: HttpResponse,
}

impl<'exec> EndHookPayload<HttpResponse> for OnSubgraphHttpResponseHookPayload<'exec> {}

pub type OnSubgraphHttpResponseHookResult<'exec> =
    crate::plugin_trait::EndHookResult<OnSubgraphHttpResponseHookPayload<'exec>, HttpResponse>;
