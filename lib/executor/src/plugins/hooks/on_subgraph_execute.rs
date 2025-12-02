use crate::{
    executors::{common::SubgraphExecutionRequest, http::HttpResponse},
    plugin_context::{PluginContext, RouterHttpRequest},
    plugin_trait::{EndHookPayload, EndHookResult, StartHookPayload, StartHookResult},
};

pub struct OnSubgraphExecuteStartHookPayload<'exec> {
    pub router_http_request: &'exec RouterHttpRequest<'exec>,
    pub context: &'exec PluginContext,

    pub subgraph_name: &'exec str,

    pub execution_request: SubgraphExecutionRequest<'exec>,
    pub execution_result: Option<HttpResponse>,
}

impl<'exec> StartHookPayload<OnSubgraphExecuteEndHookPayload<'exec>>
    for OnSubgraphExecuteStartHookPayload<'exec>
{
}

pub type OnSubgraphExecuteStartHookResult<'exec> = StartHookResult<
    'exec,
    OnSubgraphExecuteStartHookPayload<'exec>,
    OnSubgraphExecuteEndHookPayload<'exec>,
>;

pub struct OnSubgraphExecuteEndHookPayload<'exec> {
    pub execution_result: HttpResponse,
    pub context: &'exec PluginContext,
}

impl<'exec> EndHookPayload for OnSubgraphExecuteEndHookPayload<'exec> {}

pub type OnSubgraphExecuteEndHookResult<'exec> =
    EndHookResult<OnSubgraphExecuteEndHookPayload<'exec>>;
