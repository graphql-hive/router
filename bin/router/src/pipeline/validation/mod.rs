use std::hash::{Hash, Hasher};
use std::sync::Arc;

use crate::pipeline::error::PipelineError;
use crate::pipeline::parser::GraphQLParserPayload;
use crate::schema_state::SchemaState;
use crate::shared_state::RouterSharedState;
use graphql_tools::validation::validate::{validate, ValidationPlan};
use hive_router_internal::telemetry::traces::spans::graphql::GraphQLValidateSpan;
use hive_router_plan_executor::hooks::on_graphql_validation::{
    OnGraphQLValidationEndHookPayload, OnGraphQLValidationStartHookPayload,
};
use hive_router_plan_executor::hooks::on_supergraph_load::SupergraphData;
use hive_router_plan_executor::plugin_context::PluginRequestState;
use hive_router_plan_executor::plugin_trait::{EndControlFlow, StartControlFlow};
use hive_router_query_planner::consumer_schema::ConsumerSchema;
use tracing::{error, trace, Instrument};
use xxhash_rust::xxh3::Xxh3;
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
        let mut errors = None;
        let mut on_end_callbacks = vec![];
        let mut validation_schema = supergraph.planner.consumer_schema.clone();
        let mut validation_operation = parser_payload.parsed_operation.clone();
        let mut validation_rules = app_state.validation_plan.clone();

        if let Some(plugin_req_state) = plugin_req_state {
            let mut start_payload = OnGraphQLValidationStartHookPayload {
                router_http_request: &plugin_req_state.router_http_request,
                context: &plugin_req_state.context,
                schema: validation_schema.clone(),
                document: validation_operation.clone(),
                validation_plan: validation_rules.clone(),
                errors,
            };

            for plugin in plugin_req_state.plugins.iter() {
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
                        // store the callback to be called later
                        on_end_callbacks.push(callback);
                    }
                }
            }

            validation_schema = Arc::clone(&start_payload.schema);
            validation_operation = Arc::clone(&start_payload.document);
            validation_rules = Arc::clone(&start_payload.validation_plan);
            errors = start_payload.errors;
        }

        let cache_key = calculate_cache_key(
            &validation_schema,
            parser_payload.cache_key,
            &validation_rules,
        );

        let mut errors = match errors {
            Some(errors) => errors,
            None => match schema_state
                .validate_cache
                .get(&cache_key)
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

                    let res = validate(
                        &validation_schema.document,
                        &validation_operation,
                        &app_state.validation_plan,
                    );
                    let arc_res = Arc::new(res);

                    schema_state
                        .validate_cache
                        .insert(parser_payload.cache_key, arc_res.clone())
                        .await;
                    arc_res
                }
            },
        };

        if !on_end_callbacks.is_empty() {
            let mut end_payload = OnGraphQLValidationEndHookPayload {
                errors,
            };
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
            errors = end_payload.errors;
        }

        if !errors.is_empty() {
            error!(
                "GraphQL validation failed with total of {} errors",
                errors.len()
            );
            trace!("Validation errors: {:?}", errors);

            return Err(PipelineError::ValidationErrors(errors));
        }

        Ok(None)
    }
    .instrument(validate_span.clone())
    .await
}

#[inline]
fn calculate_cache_key(
    consumer_schema: &ConsumerSchema,
    document_hash: u64,
    validation_plan: &ValidationPlan,
) -> u64 {
    let mut hasher = Xxh3::new();
    consumer_schema.hash.hash(&mut hasher);
    document_hash.hash(&mut hasher);
    validation_plan.hash.hash(&mut hasher);
    hasher.finish()
}