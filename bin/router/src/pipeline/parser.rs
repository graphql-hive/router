use std::hash::{Hash, Hasher};
use std::sync::{Arc, OnceLock};

use crate::executor::hooks::on_graphql_params::GraphQLParams;
use crate::executor::hooks::on_graphql_parse::{
    OnGraphQLParseEndHookPayload, OnGraphQLParseStartHookPayload,
};
use crate::executor::plugin_context::PluginRequestState;
use crate::executor::plugin_trait::{CacheHint, EndControlFlow, StartControlFlow};
use crate::executor::plugins::hooks;
use crate::query_planner::state::supergraph_state::OperationKind;
use crate::query_planner::utils::parsing::{
    safe_parse_operation, safe_parse_operation_with_token_limit, shrink_parsed_operation,
};
use crate::telemetry::logging::targets;
use crate::telemetry::traces::spans::graphql::{GraphQLParseSpan, GraphQLSpanOperationIdentity};
use combine::easy::Info;
use graphql_tools::flat::{self, Document as FlatDocument, OperationKind as FlatOperationKind};
use graphql_tools::parser::minify_query;
use graphql_tools::parser::query::Document;
use graphql_tools::validation::utils::ValidationError;
use hive_console_sdk::agent::utils::normalize_operation as hive_sdk_normalize_operation;
use xxhash_rust::xxh3::Xxh3;

use crate::cache_state::{CacheHitMiss, EntryResultHitMissExt};
use crate::pipeline::error::{ParserCacheError, PipelineError};
use crate::pipeline::execution_request::GetQueryStr;
use crate::shared_state::RouterSharedState;
use tracing::{debug, error, warn, Instrument};

/// A parsed operation. The router works with the flat document. The tree is made only for
/// plugins and the stages not on the flat AST yet, the first time one of them asks, by parsing
/// the same text again.
#[derive(Debug)]
pub struct ParsedOperation {
    pub flat: Arc<FlatDocument>,
    /// The text the flat document was parsed from, for the tree.
    source: Box<str>,
    tree: OnceLock<Arc<Document<'static, String>>>,
}

impl ParsedOperation {
    /// Parses `source`. A document the flat parser rejects goes through the tree parser too, for
    /// the error clients have always seen.
    fn parse(source: &str, token_limit: Option<usize>) -> Result<Self, ParserCacheError> {
        let tree_error = |err: graphql_tools::parser::query::ParseError| {
            if let Some(combine::stream::easy::Error::Message(Info::Static(msg))) =
                err.0.errors.first()
            {
                if *msg == "Token limit exceeded" {
                    warn!(target: targets::GRAPHQL_PARSING, "token limit exceeded");

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

            debug!(target: targets::GRAPHQL_PARSING, error = ?err, "failed to parse GraphQL operation");

            ParserCacheError::ParseError(Arc::new(err))
        };

        match flat::parse_with_limits(source, token_limit) {
            Ok(flat) => Ok(Self {
                flat: Arc::new(flat),
                source: source.into(),
                tree: OnceLock::new(),
            }),
            Err(flat_error) => {
                let tree = match token_limit {
                    Some(limit) => safe_parse_operation_with_token_limit(source, limit),
                    None => safe_parse_operation(source),
                }
                .map_err(tree_error)?;
                // the two parsers accept the same documents, this one is a bug to look into
                error!(target: targets::GRAPHQL_PARSING, error = %flat_error, "the flat parser rejected an operation the tree parser accepts");
                Ok(Self::from_tree(Arc::new(tree)))
            }
        }
    }

    /// From a tree a plugin gave back.
    pub fn from_tree(tree: Arc<Document<'static, String>>) -> Self {
        Self {
            flat: Arc::new(crate::query_planner::ast::flat::tree::from_tree(&tree)),
            source: Box::default(),
            tree: OnceLock::from(tree),
        }
    }

    /// The tree, parsed the first time it's asked for.
    pub fn tree(&self) -> Arc<Document<'static, String>> {
        Arc::clone(self.tree.get_or_init(|| {
            let mut tree = safe_parse_operation(&self.source)
                .expect("the flat parser accepted it, so does the tree parser");
            shrink_parsed_operation(&mut tree);
            Arc::new(tree)
        }))
    }

    /// The kind and name of the first operation.
    fn operation_identity(&self) -> (OperationKind, Option<String>) {
        let doc = &self.flat;
        match doc.operations().next() {
            Some(op) => {
                let kind = match op.kind {
                    FlatOperationKind::Query | FlatOperationKind::Shorthand => OperationKind::Query,
                    FlatOperationKind::Mutation => OperationKind::Mutation,
                    FlatOperationKind::Subscription => OperationKind::Subscription,
                };
                (kind, doc.opt_str(op.name).map(str::to_string))
            }
            // the validation stage reports a document without operations
            None => (OperationKind::Query, None),
        }
    }
}

#[derive(Clone)]
pub struct ParseCacheEntry {
    document: Arc<ParsedOperation>,
    document_minified_string: Arc<String>,
    hive_operation_hash: Arc<String>,
}

#[derive(Debug)]
pub struct GraphQLParserPayload {
    pub parsed_operation: Arc<ParsedOperation>,
    pub minified_document: Arc<String>,
    pub operation_name: Option<String>,
    pub operation_type: OperationKind,
    pub cache_key: u64,
    pub cache_key_string: String,
    pub hive_operation_hash: Arc<String>,
}

impl<'a> From<&'a GraphQLParserPayload> for GraphQLSpanOperationIdentity<'a> {
    fn from(op_id: &'a GraphQLParserPayload) -> Self {
        GraphQLSpanOperationIdentity {
            name: op_id.operation_name.as_deref(),
            operation_type: op_id.operation_type.as_str(),
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
        let mut on_end_callbacks = vec![];
        if let Some(plugin_req_state) = plugin_req_state {
            let mut start_payload = OnGraphQLParseStartHookPayload {
                router_http_request: &plugin_req_state.router_http_request,
                context: &plugin_req_state.context,
                request_context: plugin_req_state
                    .request_context
                    .for_plugin::<hooks::OnGraphqlParse>(),
                graphql_params,
            };
            for plugin in plugin_req_state.plugins.as_ref() {
                let result = plugin.on_graphql_parse(start_payload).await;
                start_payload = result.payload;
                match result.control_flow {
                    StartControlFlow::Proceed => {
                        // continue to next plugin
                    }
                    StartControlFlow::EndWithResponse(response) => {
                        debug!(target: targets::GRAPHQL_PARSING, "plugin returned early response during parsing phase");

                        return Ok(ParseResult::EarlyResponse(response));
                    }
                    StartControlFlow::OnEnd(callback) => {
                        // store the callback to be called later
                        on_end_callbacks.push(callback);
                    }
                }
            }
        }

        let metrics = &app_state.telemetry_context.metrics;
        let parse_cache_capture = metrics.cache.parse.capture_request();
        let cache_key = {
            let mut hasher = Xxh3::new();
            graphql_params.query.hash(&mut hasher);
            hasher.finish()
        };

        let mut cache_hint = CacheHint::Hit;

        let query_str = graphql_params.get_query()?;

        let parse_cache_item = app_state
            .parse_cache
            .entry(cache_key)
            .or_try_insert_with::<_, ParserCacheError>(async {
                let token_limit = app_state.router_config.limits.max_tokens.as_ref().map(|cfg| cfg.n);
                let parsed = ParsedOperation::parse(query_str, token_limit)?;

                let parsed_arc = Arc::new(parsed);
                let minified_arc = Arc::new(minify_query(query_str).map_err(|err| {
                    error!(target: targets::GRAPHQL_PARSING, error = ?err, "failed to minify parsed GraphQL operation");

                    ParserCacheError::MinifyError(err.to_string())
                })?);
                // ponytail: the usage report still reads the tree, so every new operation
                // parses twice until it reads the flat document
                let hive_normalized_operation = hive_sdk_normalize_operation(&parsed_arc.tree());
                let hive_minified = minify_query(hive_normalized_operation.to_string().as_ref())
                    .map_err(|err| {
                        error!(target: targets::GRAPHQL_PARSING, error = ?err, "failed to minify GraphQL operation normalized for Hive SDK");

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
                    cache_hint = CacheHint::Miss;
                    parse_span.record_cache_hit(false);
                    parse_cache_capture.finish_miss();
                }
            })?;

        let mut parsed_operation = parse_cache_item.document;

        if !on_end_callbacks.is_empty() {
            let tree = parsed_operation.tree();
            let mut end_payload = OnGraphQLParseEndHookPayload {
                document: Arc::clone(&tree),
                cache_hint,
                request_context: plugin_req_state
                    .as_ref()
                    .map(|state| state.request_context.for_plugin::<hooks::OnGraphqlParse>())
                    .unwrap(),
            };
            for callback in on_end_callbacks {
                let result = callback(end_payload);
                end_payload = result.payload;
                match result.control_flow {
                    EndControlFlow::Proceed => {
                        // continue to next callback
                    }
                    EndControlFlow::EndWithResponse(response) => {
                      debug!(target: targets::GRAPHQL_PARSING, "plugin returned early response during parsing phase (on-end)");

                        return Ok(ParseResult::EarlyResponse(response));
                    }
                }
            }
            // a plugin may have put another document in its place
            if !Arc::ptr_eq(&tree, &end_payload.document) {
                parsed_operation = Arc::new(ParsedOperation::from_tree(end_payload.document));
            }
        }

        let cache_key_string = cache_key.to_string();

        let (operation_type, operation_name) = parsed_operation.operation_identity();

        let payload = GraphQLParserPayload {
            parsed_operation,
            minified_document: parse_cache_item.document_minified_string,
            operation_name,
            operation_type,
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
