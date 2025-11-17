use bytes::Bytes;
use hive_router_query_planner::planner::plan_nodes::FetchNode;

use crate::{executors::common::{SubgraphExecutionRequest, SubgraphExecutorBoxedArc}, plugin_trait::{EndPayload, StartPayload}, response::subgraph_response::SubgraphResponse};


pub struct OnSubgraphExecuteStartPayload<'exec> {
    pub router_http_request: &'exec ntex::web::HttpRequest,
    pub executor: &'exec SubgraphExecutorBoxedArc,
    pub subgraph_name: &'exec str,

    pub node: &'exec mut FetchNode,
    pub execution_request: &'exec mut SubgraphExecutionRequest<'exec>,
    pub response: &'exec mut Option<SubgraphExecutorResponse<'exec>>,
}   

impl<'exec> StartPayload<OnSubgraphExecuteEndPayload<'exec>> for OnSubgraphExecuteStartPayload<'exec> {}

pub enum SubgraphExecutorResponse<'exec> {
    Bytes(Bytes),
    SubgraphResponse(SubgraphResponse<'exec>),
}

pub struct OnSubgraphExecuteEndPayload<'exec> {
    pub router_http_request: &'exec ntex::web::HttpRequest,
    pub executor: &'exec SubgraphExecutorBoxedArc,
    pub subgraph_name: &'exec str,

    pub node: &'exec FetchNode,
    pub execution_request: &'exec SubgraphExecutionRequest<'exec>,
    pub response: &'exec mut SubgraphExecutorResponse<'exec>,
}

impl<'exec> EndPayload for OnSubgraphExecuteEndPayload<'exec> {}