use std::sync::Arc;

use graphql_tools::static_graphql::schema::Document;
use hive_router_internal::authorization::metadata::AuthorizationMetadata;
use hive_router_query_planner::planner::Planner;

use crate::{
    introspection::schema::SchemaMetadata,
    plugin_trait::{EndHookPayload, FromGraphQLErrorToResponse, StartHookPayload},
    response::graphql_error::GraphQLError,
    SubgraphExecutorMap,
};

pub struct SupergraphData {
    pub metadata: SchemaMetadata,
    pub planner: Planner,
    pub authorization: AuthorizationMetadata,
    pub subgraph_executor_map: SubgraphExecutorMap,
    pub supergraph_schema: Arc<Document>,
}

pub type OnSupergraphLoadResult = Result<SupergraphData, GraphQLError>;

pub struct OnSupergraphLoadStartHookPayload {
    pub current_supergraph_data: Arc<Option<SupergraphData>>,
    pub new_ast: Document,
}

impl StartHookPayload<OnSupergraphLoadEndHookPayload, OnSupergraphLoadResult>
    for OnSupergraphLoadStartHookPayload
{
}

pub type OnSupergraphLoadStartHookResult<'exec> = crate::plugin_trait::StartHookResult<
    'exec,
    OnSupergraphLoadStartHookPayload,
    OnSupergraphLoadEndHookPayload,
    OnSupergraphLoadResult,
>;

pub struct OnSupergraphLoadEndHookPayload {
    pub new_supergraph_data: SupergraphData,
}

impl EndHookPayload<OnSupergraphLoadResult> for OnSupergraphLoadEndHookPayload {}

pub type OnSupergraphLoadEndHookResult =
    crate::plugin_trait::EndHookResult<OnSupergraphLoadEndHookPayload, OnSupergraphLoadResult>;

impl FromGraphQLErrorToResponse for OnSupergraphLoadResult {
    fn from_graphql_error_to_response(error: GraphQLError, _status_code: http::StatusCode) -> Self {
        Err(error)
    }
}
