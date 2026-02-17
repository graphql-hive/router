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
use hive_router_plan_executor::hooks::on_graphql_params::GraphQLParams;
use hive_router_plan_executor::hooks::on_graphql_parse::{
    OnGraphQLParseEndHookPayload, OnGraphQLParseStartHookPayload,
};
use hive_router_plan_executor::plugin_context::PluginRequestState;
use hive_router_plan_executor::plugin_trait::{CacheHint, EndControlFlow, StartControlFlow};
use hive_router_query_planner::utils::parsing::{
    safe_parse_operation, safe_parse_operation_with_token_limit,
};
use xxhash_rust::xxh3::Xxh3;

use crate::pipeline::error::PipelineError;
use crate::pipeline::execution_request::GetQueryStr;
use crate::shared_state::RouterSharedState;
use tracing::{error, trace, Instrument};

#[derive(Clone)]
pub struct ParseCacheEntry {
    document: Arc<Document<'static, String>>,
    document_minified_string: Arc<String>,
    hive_operation_hash: Arc<String>,
}

impl ParseCacheEntry {
    pub fn try_new(
        parsed_arc: Arc<Document<'static, String>>,
        query_str: &str,
    ) -> Result<Self, PipelineError> {
        let minified_arc = {
            Arc::new(minify_query(query_str).map_err(|err| {
                error!("Failed to minify parsed GraphQL operation: {}", err);
                PipelineError::FailedToMinifyParsedOperation(err.to_string())
            })?)
        };
        let hive_normalized_operation = hive_sdk_normalize_operation(&parsed_arc);
        let hive_minified =
            minify_query(hive_normalized_operation.to_string().as_ref()).map_err(|err| {
                error!(
                    "Failed to minify GraphQL operation normalized for Hive SDK: {}",
                    err
                );
                PipelineError::FailedToMinifyParsedOperation(err.to_string())
            })?;
        Ok(ParseCacheEntry {
            document: parsed_arc,
            document_minified_string: minified_arc,
            hive_operation_hash: Arc::new(format!("{:x}", md5::compute(hive_minified))),
        })
    }
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
pub enum ParseResult {
    Payload(GraphQLParserPayload),
    EarlyResponse(ntex::http::Response),
}

#[inline]
pub async fn parse_operation_with_cache(
    app_state: &RouterSharedState,
    graphql_params: &GraphQLParams,
    plugin_req_state: &Option<PluginRequestState<'_>>,
) -> Result<ParseResult, PipelineError> {
    let parse_span = GraphQLParseSpan::new();

    async {
        let mut overridden_document = None;
        let mut on_end_callbacks = vec![];
        if let Some(plugin_req_state) = plugin_req_state {
            let mut start_payload = OnGraphQLParseStartHookPayload {
                router_http_request: &plugin_req_state.router_http_request,
                context: &plugin_req_state.context,
                graphql_params,
                document: overridden_document,
            };
            for plugin in plugin_req_state.plugins.as_ref() {
                let result = plugin.on_graphql_parse(start_payload).await;
                start_payload = result.payload;
                match result.control_flow {
                    StartControlFlow::Proceed => {
                        // continue to next plugin
                    }
                    StartControlFlow::EndWithResponse(response) => {
                        return Ok(ParseResult::EarlyResponse(response));
                    }
                    StartControlFlow::OnEnd(callback) => {
                        // store the callback to be called later
                        on_end_callbacks.push(callback);
                    }
                }
            }
            overridden_document = start_payload.document;
        }

        let query_str = graphql_params.get_query()?;

        let cache_key = {
            let mut hasher = Xxh3::new();
            query_str.hash(&mut hasher);
            hasher.finish()
        };

        let cache_hint;

        let parse_cache_item = if let Some(document) = overridden_document {
            trace!("Using overridden parsed operation from plugin");
            cache_hint = CacheHint::Miss;
            parse_span.record_cache_hit(false);
            ParseCacheEntry::try_new(document, query_str)?
        } else if let Some(cached) = app_state.parse_cache.get(&cache_key).await {
            trace!("Found cached parsed operation for query");
            cache_hint = CacheHint::Hit;
            parse_span.record_cache_hit(true);
            cached
        } else {
            cache_hint = CacheHint::Miss;
            parse_span.record_cache_hit(false);
            let parsed = match app_state.router_config.limits.max_tokens.as_ref() {
                Some(cfg) => safe_parse_operation_with_token_limit(query_str, cfg.n),
                _ => safe_parse_operation(query_str),
            }
            .map_err(|err| {
                if let Some(combine::stream::easy::Error::Message(Info::Static(msg))) =
                    err.0.errors.first()
                {
                    if *msg == "Token limit exceeded" {
                        return PipelineError::ValidationErrors(
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
                PipelineError::FailedToParseOperation(err)
            })?;
            trace!("successfully parsed GraphQL operation");
            let parsed_arc = Arc::new(parsed);
            let entry = ParseCacheEntry::try_new(parsed_arc, query_str)?;

            app_state.parse_cache.insert(cache_key, entry.clone()).await;
            entry
        };

        let mut parsed_operation = parse_cache_item.document;

        if !on_end_callbacks.is_empty() {
            let mut end_payload = OnGraphQLParseEndHookPayload {
                document: parsed_operation,
                cache_hint,
            };
            for callback in on_end_callbacks {
                let result = callback(end_payload);
                end_payload = result.payload;
                match result.control_flow {
                    EndControlFlow::Proceed => {
                        // continue to next callback
                    }
                    EndControlFlow::EndWithResponse(response) => {
                        return Ok(ParseResult::EarlyResponse(response));
                    }
                }
            }
            // Give the ownership back to variables
            parsed_operation = end_payload.document;
        }

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

        Ok(ParseResult::Payload(payload))
    }
    .instrument(parse_span.clone())
    .await
}
