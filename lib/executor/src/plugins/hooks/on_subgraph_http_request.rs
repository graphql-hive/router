use std::sync::Arc;

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

    // Early response
    pub response: Option<Arc<HttpResponse>>,
}

impl<'exec> StartHookPayload<OnSubgraphHttpResponseHookPayload<'exec>>
    for OnSubgraphHttpRequestHookPayload<'exec>
{
}

pub type OnSubgraphHttpRequestHookResult<'exec> = crate::plugin_trait::StartHookResult<
    'exec,
    OnSubgraphHttpRequestHookPayload<'exec>,
    OnSubgraphHttpResponseHookPayload<'exec>,
>;

impl<'exec> OnSubgraphHttpRequestHookPayload<'exec> {
    /// Sets the subgraph http response earlier
    /// And this skips the actual subgraph http call
    pub fn with_response(mut self, response: Arc<HttpResponse>) -> Self {
        self.response = Some(response);
        self
    }
}

pub struct OnSubgraphHttpResponseHookPayload<'exec> {
    pub context: &'exec PluginContext,
    pub response: Arc<HttpResponse>,
}

impl<'exec> EndHookPayload for OnSubgraphHttpResponseHookPayload<'exec> {}

pub type OnSubgraphHttpResponseHookResult<'exec> =
    crate::plugin_trait::EndHookResult<OnSubgraphHttpResponseHookPayload<'exec>>;
