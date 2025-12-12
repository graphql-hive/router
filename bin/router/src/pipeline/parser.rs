use std::hash::{Hash, Hasher};
use std::sync::Arc;

use graphql_parser::query::{Definition, Document, OperationDefinition};
use hive_router_internal::telemetry::traces::spans::graphql::{
    GraphQLParseSpan, GraphQLSpanOperationIdentity, RecordOperationIdentity,
};
use hive_router_query_planner::utils::parsing::safe_parse_operation;
use ntex::web::HttpRequest;
use xxhash_rust::xxh3::Xxh3;

use crate::pipeline::error::{PipelineError, PipelineErrorFromAcceptHeader, PipelineErrorVariant};
use crate::pipeline::execution_request::ExecutionRequest;
use crate::shared_state::RouterSharedState;
use tracing::{error, trace};

#[derive(Debug, Clone)]
pub struct GraphQLParserPayload {
    pub parsed_operation: Arc<Document<'static, String>>,
    pub operation_name: Option<String>,
    pub operation_type: String,
    pub cache_key: u64,
    pub cache_key_string: String,
}

impl<'a> From<&'a GraphQLParserPayload> for GraphQLSpanOperationIdentity<'a> {
    fn from(op_id: &'a GraphQLParserPayload) -> Self {
        GraphQLSpanOperationIdentity {
            name: op_id.operation_name.as_deref(),
            operation_type: &op_id.operation_type,
            client_document_hash: &op_id.cache_key_string,
        }
    }
}

#[inline]
pub async fn parse_operation_with_cache(
    req: &HttpRequest,
    app_state: &Arc<RouterSharedState>,
    execution_params: &ExecutionRequest,
) -> Result<GraphQLParserPayload, PipelineError> {
    let parse_span = GraphQLParseSpan::new();
    let _guard = parse_span.span.enter();
    let cache_key = {
        let mut hasher = Xxh3::new();
        execution_params.query.hash(&mut hasher);
        hasher.finish()
    };

    let parsed_operation = if let Some(cached) = app_state.parse_cache.get(&cache_key).await {
        trace!("Found cached parsed operation for query");
        parse_span.record_cache_hit(true);
        cached
    } else {
        parse_span.record_cache_hit(false);
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

    let cache_key_string = cache_key.to_string();

    let (operation_type, operation_name) =
        match parsed_operation
            .definitions
            .iter()
            .find_map(|def| match def {
                Definition::Operation(op) => Some(op),
                _ => None,
            }) {
            Some(OperationDefinition::Query(def)) => {
                ("query", def.name.as_ref().map(|s| s.to_string()))
            }
            Some(OperationDefinition::Mutation(def)) => {
                ("mutation", def.name.as_ref().map(|s| s.to_string()))
            }
            Some(OperationDefinition::Subscription(def)) => {
                ("subscription", def.name.as_ref().map(|s| s.to_string()))
            }
            Some(OperationDefinition::SelectionSet(_)) => ("query", None),
            None => {
                // This should not happen as we must have at least one operation definition
                // but just in case, we handle it gracefully,
                // the error will be caught later in the pipeline, specifically in the validation stage
                ("query", None)
            }
        };

    let payload = GraphQLParserPayload {
        parsed_operation,
        operation_name,
        operation_type: operation_type.to_string(),
        cache_key,
        cache_key_string,
    };

    parse_span.record_operation_identity((&payload).into());

    Ok(payload)
}
