use crate::{
    executors::common::{SubgraphExecutionRequest, SubgraphExecutorBoxedArc},
    plugin_context::{PluginContext, RouterHttpRequest},
    plugin_trait::{EndHookPayload, EndHookResult, StartHookPayload, StartHookResult},
    response::subgraph_response::SubgraphResponse,
};

pub struct OnSubgraphExecuteStartHookPayload<'exec> {
    pub router_http_request: &'exec RouterHttpRequest<'exec>,
    pub context: &'exec PluginContext,

    pub subgraph_name: &'exec str,
    pub executor: SubgraphExecutorBoxedArc,

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
    pub execution_result: SubgraphResponse<'exec>,
    pub context: &'exec PluginContext,
}

impl<'exec> EndHookPayload<SubgraphResponse<'exec>> for OnSubgraphExecuteEndHookPayload<'exec> {}

pub type OnSubgraphExecuteEndHookResult<'exec> =
    EndHookResult<OnSubgraphExecuteEndHookPayload<'exec>, SubgraphResponse<'exec>>;
