use crate::{
    executors::common::{SubgraphExecutionRequest, SubgraphExecutorBoxedArc},
    plugin_context::{PluginContext, RouterHttpRequest},
    plugin_trait::{EndHookPayload, EndHookResult, StartHookPayload, StartHookResult},
    response::subgraph_response::SubgraphResponse,
};

pub struct OnSubgraphExecuteStartHookPayload<'exec, 'req> {
    pub router_http_request: &'req RouterHttpRequest<'req>,
    pub context: &'exec PluginContext,

    pub subgraph_name: &'exec str,
    pub executor: SubgraphExecutorBoxedArc,

    pub execution_request: SubgraphExecutionRequest<'exec>,
    // Override
    pub execution_result: Option<SubgraphResponse<'exec>>,
}

impl<'exec> OnSubgraphExecuteStartHookPayload<'exec, '_> {
    pub fn with_execution_result(mut self, execution_result: SubgraphResponse<'exec>) -> Self {
        self.execution_result = Some(execution_result);
        self
    }
}

impl<'exec, 'req> StartHookPayload<OnSubgraphExecuteEndHookPayload<'exec>>
    for OnSubgraphExecuteStartHookPayload<'exec, 'req>
{
}

pub type OnSubgraphExecuteStartHookResult<'exec, 'req> = StartHookResult<
    'exec,
    OnSubgraphExecuteStartHookPayload<'exec, 'req>,
    OnSubgraphExecuteEndHookPayload<'exec>,
>;

pub struct OnSubgraphExecuteEndHookPayload<'exec> {
    pub execution_result: SubgraphResponse<'exec>,
    pub context: &'exec PluginContext,
}

impl<'exec> EndHookPayload for OnSubgraphExecuteEndHookPayload<'exec> {}

pub type OnSubgraphExecuteEndHookResult<'exec> =
    EndHookResult<OnSubgraphExecuteEndHookPayload<'exec>>;
