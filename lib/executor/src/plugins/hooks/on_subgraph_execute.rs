use crate::{
    executors::common::{HttpExecutionResponse, SubgraphExecutionRequest},
    plugin_context::{PluginContext, RouterHttpRequest},
    plugin_trait::{EndPayload, StartPayload},
};

pub struct OnSubgraphExecuteStartPayload<'exec> {
    pub router_http_request: &'exec RouterHttpRequest<'exec>,
    pub context: &'exec PluginContext,

    pub subgraph_name: String,

    pub execution_request: SubgraphExecutionRequest<'exec>,
    pub execution_result: Option<HttpExecutionResponse>,
}

impl<'exec> StartPayload<OnSubgraphExecuteEndPayload> for OnSubgraphExecuteStartPayload<'exec> {}

pub struct OnSubgraphExecuteEndPayload {
    pub execution_result: HttpExecutionResponse,
}

impl EndPayload for OnSubgraphExecuteEndPayload {}
