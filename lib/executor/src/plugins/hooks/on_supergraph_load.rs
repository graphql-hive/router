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
    /// The metadata of the supergraph schema,
    /// which includes the list of subgraphs, their relationships, and other relevant information about the supergraph.
    pub metadata: SchemaMetadata,
    /// The query planner instance that will be used to generate the query plan for the incoming GraphQL requests based on the supergraph schema.
    pub planner: Planner,
    /// The authorization metadata that will be used to authorize the incoming GraphQL requests based on the supergraph schema and the authorization rules defined in the router.
    pub authorization: AuthorizationMetadata,
    /// The map of subgraph executors that will be used to execute the query plan for the incoming GraphQL requests based on the supergraph schema.
    pub subgraph_executor_map: SubgraphExecutorMap,
    /// The AST of the supergraph schema document that was loaded and parsed by the router.
    pub supergraph_schema: Arc<Document>,
}

pub type OnSupergraphLoadResult = Result<SupergraphData, GraphQLError>;

pub struct OnSupergraphLoadStartHookPayload {
    /// The current supergraph data that is currently used by the router before loading the new supergraph schema.
    pub current_supergraph_data: Arc<Option<SupergraphData>>,
    /// The raw SDL string of the new supergraph schema that is being loaded by the router.
    /// Plugins can modify the SDL string before it is parsed and loaded by the router,
    /// and the modified SDL string will be used in the loading process instead of the original one.
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
    /// The new supergraph data that is generated from loading the new supergraph schema.
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
