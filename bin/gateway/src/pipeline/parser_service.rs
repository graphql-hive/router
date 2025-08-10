use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;

use graphql_parser::query::Document;
use ntex::web::HttpRequest;
use query_planner::utils::parsing::safe_parse_operation;

use crate::pipeline::error::{PipelineError, PipelineErrorFromAcceptHeader, PipelineErrorVariant};
use crate::pipeline::graphql_request_params::ExecutionRequest;
use crate::shared_state::GatewaySharedState;
use tracing::{error, trace};

#[derive(Debug, Clone)]
pub struct GraphQLParserPayload {
    pub parsed_operation: Arc<Document<'static, String>>,
    pub cache_key: u64,
}

#[inline]
pub async fn parse_operation(
    req: &HttpRequest,
    execution_params: &ExecutionRequest,
    state: &GatewaySharedState,
) -> Result<GraphQLParserPayload, PipelineError> {
    let cache_key = {
        let mut hasher = DefaultHasher::new();
        execution_params.query.hash(&mut hasher);
        hasher.finish()
    };

    let parsed_operation = if let Some(cached) = state.parse_cache.get(&cache_key).await {
        trace!("Found cached parsed operation for query");
        cached
    } else {
        let parsed = safe_parse_operation(&execution_params.query).map_err(|err| {
            error!("Failed to parse GraphQL operation: {}", err);
            req.new_pipeline_error(PipelineErrorVariant::FailedToParseOperation(err))
        })?;
        trace!("sucessfully parsed GraphQL operation");
        let parsed_arc = Arc::new(parsed);
        state
            .parse_cache
            .insert(cache_key, parsed_arc.clone())
            .await;
        parsed_arc
    };

    Ok(GraphQLParserPayload {
        parsed_operation,
        cache_key,
    })
}
