use std::sync::Arc;

use arc_swap::ArcSwap;
use graphql_tools::static_graphql::schema::Document;
use hive_router_internal::authorization::metadata::AuthorizationMetadata;
use hive_router_query_planner::planner::Planner;

use crate::{
    introspection::schema::SchemaMetadata,
    plugin_trait::{EndHookPayload, StartHookPayload},
    SubgraphExecutorMap,
};

pub struct SupergraphData {
    pub metadata: SchemaMetadata,
    pub planner: Planner,
    pub authorization: AuthorizationMetadata,
    pub subgraph_executor_map: SubgraphExecutorMap,
    pub supergraph_schema: Arc<Document>,
}

pub struct OnSupergraphLoadStartHookPayload {
    pub current_supergraph_data: Arc<ArcSwap<Option<SupergraphData>>>,
    pub new_ast: Document,
}

impl StartHookPayload<OnSupergraphLoadEndHookPayload> for OnSupergraphLoadStartHookPayload {}

pub type OnSupergraphLoadStartHookResult<'exec> = crate::plugin_trait::StartHookResult<
    'exec,
    OnSupergraphLoadStartHookPayload,
    OnSupergraphLoadEndHookPayload,
>;

pub struct OnSupergraphLoadEndHookPayload {
    pub new_supergraph_data: SupergraphData,
}

impl EndHookPayload for OnSupergraphLoadEndHookPayload {}

pub type OnSupergraphLoadEndHookResult =
    crate::plugin_trait::EndHookResult<OnSupergraphLoadEndHookPayload>;
