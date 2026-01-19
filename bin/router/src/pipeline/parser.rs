use std::hash::{Hash, Hasher};
use std::sync::Arc;

use graphql_tools::parser::minify_query;
use graphql_tools::parser::query::{Definition, Document, OperationDefinition};
use hive_console_sdk::agent::utils::normalize_operation as hive_sdk_normalize_operation;
use hive_router_internal::telemetry::traces::spans::graphql::{
    GraphQLParseSpan, GraphQLSpanOperationIdentity,
};
use hive_router_query_planner::utils::parsing::safe_parse_operation;
use xxhash_rust::xxh3::Xxh3;

use crate::pipeline::error::PipelineError;
use crate::pipeline::execution_request::ExecutionRequest;
use crate::shared_state::RouterSharedState;
use tracing::{error, trace, Instrument};

#[derive(Clone)]
pub struct ParseCacheEntry {
    document: Arc<Document<'static, String>>,
    document_minified_string: Arc<String>,
    hive_operation_hash: Arc<String>,
}

#[derive(Debug, Clone)]
pub struct GraphQLParserPayload {
    pub parsed_operation: Arc<Document<'static, String>>,
    pub minified_document: Arc<String>,
    pub operation_name: Option<String>,
    pub operation_type: String,
    pub cache_key: u64,
    pub cache_key_string: String,
    pub hive_operation_hash: Arc<String>,
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
    app_state: &RouterSharedState,
    execution_params: &ExecutionRequest,
) -> Result<GraphQLParserPayload, PipelineError> {
    let parse_span = GraphQLParseSpan::new();

    async {
        let cache_key = {
            let mut hasher = Xxh3::new();
            execution_params.query.hash(&mut hasher);
            hasher.finish()
        };

        let parse_cache_item = if let Some(cached) = app_state.parse_cache.get(&cache_key).await {
            trace!("Found cached parsed operation for query");
            parse_span.record_cache_hit(true);
            cached
        } else {
            parse_span.record_cache_hit(false);
            let parsed = safe_parse_operation(&execution_params.query).map_err(|err| {
                error!("Failed to parse GraphQL operation: {}", err);
                PipelineError::FailedToParseOperation(err)
            })?;
            trace!("sucessfully parsed GraphQL operation");

            let parsed_arc = Arc::new(parsed);
            let minified_arc = {
                Arc::new(
                    minify_query(execution_params.query.as_str()).map_err(|err| {
                        error!("Failed to minify parsed GraphQL operation: {}", err);
                        PipelineError::FailedToMinifyParsedOperation(err.to_string())
                    })?,
                )
            };

            let hive_normalized_operation = hive_sdk_normalize_operation(&parsed_arc);
            let hive_minified = minify_query(hive_normalized_operation.to_string().as_ref())
                .map_err(|err| {
                    error!(
                        "Failed to minify GraphQL operation normalized for Hive SDK: {}",
                        err
                    );
                    PipelineError::FailedToMinifyParsedOperation(err.to_string())
                })?;

            let entry = ParseCacheEntry {
                document: parsed_arc,
                document_minified_string: minified_arc,
                hive_operation_hash: Arc::new(format!("{:x}", md5::compute(hive_minified))),
            };

            app_state.parse_cache.insert(cache_key, entry.clone()).await;
            entry
        };

        let parsed_operation = parse_cache_item.document;

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
            minified_document: parse_cache_item.document_minified_string,
            operation_name,
            operation_type: operation_type.to_string(),
            cache_key,
            cache_key_string,
            hive_operation_hash: parse_cache_item.hive_operation_hash.clone(),
        };

        parse_span.record_operation_identity((&payload).into());

        Ok(payload)
    }
    .instrument(parse_span.clone())
    .await
}
