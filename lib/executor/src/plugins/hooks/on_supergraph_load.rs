use std::sync::Arc;

use arc_swap::ArcSwap;
use graphql_tools::static_graphql::schema::Document;
use hive_router_query_planner::planner::Planner;

use crate::{
    introspection::schema::SchemaMetadata,
    plugin_trait::{EndPayload, StartPayload},
    SubgraphExecutorMap,
};

pub struct SupergraphData {
    pub metadata: SchemaMetadata,
    pub planner: Planner,
    pub subgraph_executor_map: SubgraphExecutorMap,
}

pub struct OnSupergraphLoadStartPayload {
    pub current_supergraph_data: Arc<ArcSwap<Option<SupergraphData>>>,
    pub new_ast: Document,
}

impl StartPayload<OnSupergraphLoadEndPayload> for OnSupergraphLoadStartPayload {}

pub struct OnSupergraphLoadEndPayload {
    pub new_supergraph_data: SupergraphData,
}

impl EndPayload for OnSupergraphLoadEndPayload {}
