use std::hash::{Hash, Hasher};
use std::sync::Arc;

use graphql_tools::parser::query::Document;
use hive_router_query_planner::utils::parsing::{
    safe_parse_operation, safe_parse_operation_with_limit,
};
use xxhash_rust::xxh3::Xxh3;

use crate::pipeline::error::PipelineError;
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
    app_state: &RouterSharedState,
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
        let parsed = match app_state.router_config.limits.max_tokens.as_ref() {
            Some(cfg) if cfg.is_enabled() => {
                safe_parse_operation_with_limit(&execution_params.query, cfg.n, cfg.expose_limits)
            }
            _ => safe_parse_operation(&execution_params.query),
        }
        .map_err(|err| {
            error!("Failed to parse GraphQL operation: {}", err);
            PipelineError::FailedToParseOperation(err)
        })?;
        trace!("successfully parsed GraphQL operation");
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
