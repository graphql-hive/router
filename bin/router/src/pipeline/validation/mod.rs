use std::sync::Arc;

use crate::pipeline::error::PipelineError;
use crate::pipeline::parser::GraphQLParserPayload;
use crate::schema_state::SchemaState;
use crate::shared_state::RouterSharedState;
use graphql_tools::validation::validate::validate;
use hive_router_internal::telemetry::traces::spans::graphql::GraphQLValidateSpan;
use hive_router_plan_executor::hooks::on_graphql_validation::{
    OnGraphQLValidationEndHookPayload, OnGraphQLValidationStartHookPayload,
};
use hive_router_plan_executor::hooks::on_supergraph_load::SupergraphData;
use hive_router_plan_executor::plugin_context::PluginRequestState;
use hive_router_plan_executor::plugin_trait::{EndControlFlow, StartControlFlow};
use tracing::{error, trace, Instrument};
pub mod max_aliases_rule;
pub mod max_depth_rule;
pub mod max_directives_rule;
mod shared;

#[inline]
pub async fn validate_operation_with_cache(
    supergraph: &SupergraphData,
    schema_state: &SchemaState,
    app_state: &RouterSharedState,
    parser_payload: &GraphQLParserPayload,
    plugin_req_state: &Option<PluginRequestState<'_>>,
) -> Result<Option<ntex::http::Response>, PipelineError> {
    let validate_span = GraphQLValidateSpan::new();

    async {
        let consumer_schema_ast = supergraph.planner.consumer_schema.document.clone();

        let validation_result = match schema_state
            .validate_cache
            .get(&parser_payload.cache_key)
            .await
        {
            Some(cached_validation) => {
                trace!(
                    "validation result of hash {} has been loaded from cache",
                    parser_payload.cache_key
                );
                validate_span.record_cache_hit(true);

                cached_validation
            }
            None => {
                trace!(
                    "validation result of hash {} does not exists in cache",
                    parser_payload.cache_key
                );
                validate_span.record_cache_hit(false);

                let mut on_end_callbacks = vec![];
                let document = parser_payload.parsed_operation.clone();
                let mut errors = if let Some(plugin_req_state) = plugin_req_state.as_ref() {
                    /* Handle on_graphql_validate hook in the plugins - START */
                    let mut start_payload = OnGraphQLValidationStartHookPayload::new(
                        plugin_req_state,
                        consumer_schema_ast,
                        document,
                        &app_state.validation_plan,
                    );
                    for plugin in plugin_req_state.plugins.as_ref() {
                        let result = plugin.on_graphql_validation(start_payload).await;
                        start_payload = result.payload;
                        match result.control_flow {
                            StartControlFlow::Proceed => {
                                // continue to next plugin
                            }
                            StartControlFlow::EndWithResponse(response) => {
                                return Ok(Some(response));
                            }
                            StartControlFlow::OnEnd(callback) => {
                                on_end_callbacks.push(callback);
                            }
                        }
                    }
                    match start_payload.errors {
                        Some(errors) => errors,
                        None => validate(
                            &start_payload.schema,
                            &start_payload.document,
                            start_payload.get_validation_plan(),
                        ),
                    }
                } else {
                    validate(&consumer_schema_ast, &document, &app_state.validation_plan)
                };

                if !on_end_callbacks.is_empty() {
                    let mut end_payload = OnGraphQLValidationEndHookPayload { errors };

                    for callback in on_end_callbacks {
                        let result = callback(end_payload);
                        end_payload = result.payload;
                        match result.control_flow {
                            EndControlFlow::Proceed => {
                                // continue to next callback
                            }
                            EndControlFlow::EndWithResponse(response) => {
                                return Ok(Some(response));
                            }
                        }
                    }

                    // Give the ownership back to variables
                    errors = end_payload.errors;
                }

                /* Handle on_graphql_validate hook in the plugins - END */

                let arc_res = Arc::new(errors);

                schema_state
                    .validate_cache
                    .insert(parser_payload.cache_key, arc_res.clone())
                    .await;
                arc_res
            }
        };

        if !validation_result.is_empty() {
            error!(
                "GraphQL validation failed with total of {} errors",
                validation_result.len()
            );
            trace!("Validation errors: {:?}", validation_result);

            return Err(PipelineError::ValidationErrors(validation_result));
        }

        Ok(None)
    }
    .instrument(validate_span.clone())
    .await
}
