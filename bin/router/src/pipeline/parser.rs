use std::hash::{Hash, Hasher};
use std::sync::Arc;

use graphql_parser::query::Document;
use hive_router_plan_executor::execution::plan::PlanExecutionOutput;
use hive_router_plan_executor::hooks::on_graphql_params::GraphQLParams;
use hive_router_plan_executor::hooks::on_graphql_parse::{OnGraphQLParseEndPayload, OnGraphQLParseStartPayload};
use hive_router_plan_executor::plugin_trait::ControlFlowResult;
use hive_router_query_planner::utils::parsing::safe_parse_operation;
use ntex::web::HttpRequest;
use xxhash_rust::xxh3::Xxh3;

use crate::pipeline::deserialize_graphql_params::GetQueryStr;
use crate::pipeline::error::{PipelineError, PipelineErrorFromAcceptHeader, PipelineErrorVariant};
use crate::shared_state::RouterSharedState;
use tracing::{error, trace};

#[derive(Debug, Clone)]
pub struct GraphQLParserPayload {
    pub parsed_operation: Arc<Document<'static, String>>,
    pub cache_key: u64,
}

pub enum ParseResult {
    Payload(GraphQLParserPayload),
    Response(PlanExecutionOutput),
}

#[inline]
pub async fn parse_operation_with_cache(
    req: &HttpRequest,
    app_state: &Arc<RouterSharedState>,
    graphql_params: &GraphQLParams,
) -> Result<ParseResult, PipelineError> {
    let cache_key = {
        let mut hasher = Xxh3::new();
        graphql_params.query.hash(&mut hasher);
        hasher.finish()
    };

    let parsed_operation = if let Some(cached) = app_state.parse_cache.get(&cache_key).await {
        trace!("Found cached parsed operation for query");
        cached
    } else {
        /* Handle on_graphql_parse hook in the plugins - START */
        let mut start_payload = OnGraphQLParseStartPayload {
            router_http_request: req,
            graphql_params,
            document: None,
        };
        let mut on_end_callbacks = vec![];
        for plugin in app_state.plugins.as_ref() {
            let result = plugin.on_graphql_parse(start_payload);
            start_payload = result.payload;
            match result.control_flow {
                ControlFlowResult::Continue => {
                    // continue to next plugin
                }
                ControlFlowResult::EndResponse(response) => {
                    return Ok(ParseResult::Response(response));
                }
                ControlFlowResult::OnEnd(callback) => {
                    // store the callback to be called later
                    on_end_callbacks.push(callback);
                }
            }
        }
        let document = match start_payload.document {
            Some(parsed) => parsed,
            None => {
                let query_str = graphql_params.get_query().map_err(|err| {
                    req.new_pipeline_error(err)
                })?;
                let parsed = safe_parse_operation(query_str).map_err(|err| {
                    error!("Failed to parse GraphQL operation: {}", err);
                    req.new_pipeline_error(PipelineErrorVariant::FailedToParseOperation(err))
                })?;
                trace!("successfully parsed GraphQL operation");
                parsed
            }
        };
        let mut end_payload = OnGraphQLParseEndPayload {
            router_http_request: req,
            graphql_params,
            document,
        };
        for callback in on_end_callbacks {
            let result = callback(end_payload);
            end_payload = result.payload;
            match result.control_flow {
                ControlFlowResult::Continue => {
                    // continue to next callback
                }
                ControlFlowResult::EndResponse(response) => {
                    return Ok(ParseResult::Response(response));
                }
                ControlFlowResult::OnEnd(_) => {
                    // on_end callbacks should not return OnEnd again
                    unreachable!();
                }
            }
        }
        let document = end_payload.document;
        /* Handle on_graphql_parse hook in the plugins - END */

        let parsed_arc = Arc::new(document);
        app_state
            .parse_cache
            .insert(cache_key, parsed_arc.clone())
            .await;
        parsed_arc
    };

    Ok(
        ParseResult::Payload(GraphQLParserPayload {
            parsed_operation,
            cache_key,
        })
    )
}
