use crate::{
    executors::common::{HttpExecutionResponse, SubgraphExecutionRequest},
    plugin_context::{PluginContext, RouterHttpRequest},
    plugin_trait::{EndPayload, StartPayload},
};

pub struct OnSubgraphExecuteStartPayload<'exec> {
    pub router_http_request: &'exec RouterHttpRequest<'exec>,
    pub context: &'exec PluginContext,

    pub subgraph_name: &'exec str,

    pub execution_request: SubgraphExecutionRequest<'exec>,
    pub execution_result: Option<HttpExecutionResponse>,
}

impl<'exec> StartPayload<OnSubgraphExecuteEndPayload<'exec>>
    for OnSubgraphExecuteStartPayload<'exec>
{
}

pub struct OnSubgraphExecuteEndPayload<'exec> {
    pub execution_result: HttpExecutionResponse,
    pub context: &'exec PluginContext,
}

impl<'exec> EndPayload for OnSubgraphExecuteEndPayload<'exec> {}
