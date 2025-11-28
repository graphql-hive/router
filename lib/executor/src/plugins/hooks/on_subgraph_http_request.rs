use crate::{
    executors::{common::SubgraphExecutionRequest, dedupe::SharedResponse},
    plugin_context::PluginContext,
    plugin_trait::{EndPayload, StartPayload},
};

pub struct OnSubgraphHttpRequestPayload<'exec> {
    pub subgraph_name: &'exec str,

    pub endpoint: &'exec http::Uri,
    pub method: http::Method,
    pub body: Vec<u8>,
    pub execution_request: SubgraphExecutionRequest<'exec>,

    pub context: &'exec PluginContext,

    // Early response
    pub response: Option<SharedResponse>,
}

impl<'exec> StartPayload<OnSubgraphHttpResponsePayload> for OnSubgraphHttpRequestPayload<'exec> {}

pub struct OnSubgraphHttpResponsePayload {
    pub response: SharedResponse,
}

impl EndPayload for OnSubgraphHttpResponsePayload {}
