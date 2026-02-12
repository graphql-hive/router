use std::hash::{Hash, Hasher};
use std::sync::Arc;

use combine::easy::Info;
use graphql_tools::parser::minify_query;
use graphql_tools::parser::query::{Definition, Document, OperationDefinition};
use graphql_tools::validation::utils::ValidationError;
use hive_console_sdk::agent::utils::normalize_operation as hive_sdk_normalize_operation;
use hive_router_internal::telemetry::traces::spans::graphql::{
    GraphQLParseSpan, GraphQLSpanOperationIdentity,
};
use hive_router_query_planner::utils::parsing::{
    safe_parse_operation, safe_parse_operation_with_token_limit,
};
use xxhash_rust::xxh3::Xxh3;

use crate::cache_state::{CacheHitMiss, EntryResultHitMissExt};
use crate::pipeline::error::{ParserCacheError, PipelineError};
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
    let metrics = &app_state.telemetry_context.metrics;
    let parse_cache_capture = metrics.cache.parse.capture_request();
    let parse_span = GraphQLParseSpan::new();

    async {
        let cache_key = {
            let mut hasher = Xxh3::new();
            execution_params.query.hash(&mut hasher);
            hasher.finish()
        };

        let parse_cache_item = app_state
            .parse_cache
            .entry(cache_key)
            .or_try_insert_with::<_, ParserCacheError>(async {
                let parsed = match app_state.router_config.limits.max_tokens.as_ref() {
                    Some(cfg) => {
                        safe_parse_operation_with_token_limit(&execution_params.query, cfg.n)
                    }
                    _ => safe_parse_operation(&execution_params.query),
                }
                .map_err(|err| {
                    if let Some(combine::stream::easy::Error::Message(Info::Static(msg))) =
                        err.0.errors.first()
                    {
                        if *msg == "Token limit exceeded" {
                            return ParserCacheError::ValidationErrors(
                                vec![ValidationError {
                                    locations: vec![err.0.position],
                                    message: "Token limit exceeded.".to_string(),
                                    error_code: "TOKEN_LIMIT_EXCEEDED",
                                }]
                                .into(),
                            );
                        }
                    }
                    error!("Failed to parse GraphQL operation: {}", err);
                    ParserCacheError::ParseError(Arc::new(err))
                })?;
                trace!("successfully parsed GraphQL operation");
                let parsed_arc = Arc::new(parsed);
                let minified_arc = Arc::new(
                    minify_query(execution_params.query.as_str()).map_err(|err| {
                        error!("Failed to minify parsed GraphQL operation: {}", err);
                        ParserCacheError::MinifyError(err.to_string())
                    })?,
                );
                let hive_normalized_operation = hive_sdk_normalize_operation(&parsed_arc);
                let hive_minified = minify_query(hive_normalized_operation.to_string().as_ref())
                    .map_err(|err| {
                        error!(
                            "Failed to minify GraphQL operation normalized for Hive SDK: {}",
                            err
                        );
                        ParserCacheError::MinifyError(err.to_string())
                    })?;

                Ok(ParseCacheEntry {
                    document: parsed_arc,
                    document_minified_string: minified_arc,
                    hive_operation_hash: Arc::new(format!("{:x}", md5::compute(hive_minified))),
                })
            })
            .await
            .map_err(PipelineError::from)
            .into_result_with_hit_miss(|hit_miss| match hit_miss {
                CacheHitMiss::Hit => {
                    parse_span.record_cache_hit(true);
                    parse_cache_capture.finish_hit();
                }
                CacheHitMiss::Miss | CacheHitMiss::Error => {
                    parse_span.record_cache_hit(false);
                    parse_cache_capture.finish_miss();
                }
            })?;

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
