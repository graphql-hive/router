use std::hash::{Hash, Hasher};
use std::sync::Arc;

use graphql_parser::query::Document;
use ntex::web::HttpRequest;
use query_planner::utils::parsing::safe_parse_operation;
use xxhash_rust::xxh3::Xxh3;

use crate::pipeline::error::{PipelineError, PipelineErrorFromAcceptHeader, PipelineErrorVariant};
use crate::pipeline::execution_request::ExecutionRequest;
use crate::shared_state::RouterSharedState;
use tracing::{error, trace};

#[derive(Debug, Clone)]
pub struct GraphQLParserPayload {
    pub parsed_operation: Arc<Document<'static, String>>,
    pub cache_key: u64,
}

#[inline]
pub async fn parse_operation_with_cache(
    req: &HttpRequest,
    app_state: &Arc<RouterSharedState>,
    execution_params: &ExecutionRequest,
) -> Result<GraphQLParserPayload, PipelineError> {
    let cache_key = {
        let mut hasher = Xxh3::new();
        execution_params.query.hash(&mut hasher);
        hasher.finish()
    };

    let parsed_operation = if let Some(cached) = app_state.parse_cache.get(&cache_key).await {
        trace!("Found cached parsed operation for query");
        cached
    } else {
        let parsed = safe_parse_operation(&execution_params.query).map_err(|err| {
            error!("Failed to parse GraphQL operation: {}", err);
            req.new_pipeline_error(PipelineErrorVariant::FailedToParseOperation(err))
        })?;
        trace!("sucessfully parsed GraphQL operation");
        let parsed_arc = Arc::new(parsed);
        app_state
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
