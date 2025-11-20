use std::sync::Arc;

use crate::pipeline::error::PipelineErrorVariant;
use crate::pipeline::parser::GraphQLParserPayload;
use crate::schema_state::SchemaState;
use crate::shared_state::RouterSharedState;
use graphql_tools::validation::validate::validate;
use hive_router_plan_executor::execution::plan::PlanExecutionOutput;
use hive_router_plan_executor::hooks::on_graphql_validation::{
    OnGraphQLValidationEndPayload, OnGraphQLValidationStartPayload,
};
use hive_router_plan_executor::hooks::on_supergraph_load::SupergraphData;
use hive_router_plan_executor::plugin_context::PluginManager;
use hive_router_plan_executor::plugin_trait::ControlFlowResult;
use tracing::{error, trace};

#[inline]
pub async fn validate_operation_with_cache(
    supergraph: &SupergraphData,
    schema_state: Arc<SchemaState>,
    app_state: Arc<RouterSharedState>,
    parser_payload: &GraphQLParserPayload,
    plugin_manager: &PluginManager<'_>,
) -> Result<Option<PlanExecutionOutput>, PipelineErrorVariant> {
    let consumer_schema_ast = &supergraph.planner.consumer_schema.document;

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

            cached_validation
        }
        None => {
            trace!(
                "validation result of hash {} does not exists in cache",
                parser_payload.cache_key
            );

            /* Handle on_graphql_validate hook in the plugins - START */
            let mut start_payload = OnGraphQLValidationStartPayload::new(
                plugin_manager,
                consumer_schema_ast,
                &parser_payload.parsed_operation,
                &app_state.validation_plan,
            );
            let mut on_end_callbacks = vec![];
            for plugin in app_state.plugins.as_ref() {
                let result = plugin.on_graphql_validation(start_payload).await;
                start_payload = result.payload;
                match result.control_flow {
                    ControlFlowResult::Continue => {
                        // continue to next plugin
                    }
                    ControlFlowResult::EndResponse(response) => {
                        return Ok(Some(response));
                    }
                    ControlFlowResult::OnEnd(callback) => {
                        on_end_callbacks.push(callback);
                    }
                }
            }

            let errors = match start_payload.errors {
                Some(errors) => errors,
                None => validate(
                    consumer_schema_ast,
                    start_payload.document,
                    start_payload.get_validation_plan(),
                ),
            };

            let mut end_payload = OnGraphQLValidationEndPayload { errors };

            for callback in on_end_callbacks {
                let result = callback(end_payload);
                end_payload = result.payload;
                match result.control_flow {
                    ControlFlowResult::Continue => {
                        // continue to next callback
                    }
                    ControlFlowResult::EndResponse(response) => {
                        return Ok(Some(response));
                    }
                    ControlFlowResult::OnEnd(_) => {
                        // on_end callbacks should not return OnEnd again
                    }
                }
            }
            /* Handle on_graphql_validate hook in the plugins - END */

            let arc_res = Arc::new(end_payload.errors);

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

        return Err(PipelineErrorVariant::ValidationErrors(validation_result));
    }

    Ok(None)
}
