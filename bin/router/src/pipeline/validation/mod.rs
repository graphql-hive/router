use std::hash::{Hash, Hasher};
use std::sync::Arc;

use crate::cache_state::{CacheHitMiss, EntryValueHitMissExt};
use crate::pipeline::error::PipelineError;
use crate::pipeline::parser::GraphQLParserPayload;
use crate::shared_state::RouterSharedState;
use crate::SchemaState;
use graphql_tools::validation::validate::validate;
use hive_router_internal::telemetry::traces::spans::graphql::GraphQLValidateSpan;
use hive_router_plan_executor::hooks::on_graphql_validation::{
    OnGraphQLValidationEndHookPayload, OnGraphQLValidationStartHookPayload,
};
use hive_router_plan_executor::hooks::on_supergraph_load::SupergraphData;
use hive_router_plan_executor::plugin_context::PluginRequestState;
use hive_router_plan_executor::plugin_trait::{CacheHint, EndControlFlow, StartControlFlow};
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
        let mut validation_plan = app_state.validation_plan.clone();

        if let Some(plugin_req_state) = plugin_req_state {
            let mut start_payload = OnGraphQLValidationStartHookPayload {
                router_http_request: &plugin_req_state.router_http_request,
                context: &plugin_req_state.context,
                schema: validation_schema,
                document: validation_operation,
                validation_plan,
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

            validation_schema = start_payload.schema;
            validation_operation = start_payload.document;
            validation_plan = start_payload.validation_plan;
            errors = start_payload.errors;
        }

        let cache_key = {
            let mut hasher = Xxh3::new();
            validation_schema.hash.hash(&mut hasher);
            validation_plan.hash.hash(&mut hasher);
            parser_payload.cache_key.hash(&mut hasher);
            hasher.finish()
        };

        let mut cache_hint = CacheHint::Hit;

        let mut errors = match errors {
            Some(errors) => errors,
            None => {
                let metrics = &schema_state.telemetry_context.metrics;
                let validate_cache_capture = metrics.cache.validate.capture_request();
                schema_state
                    .validate_cache
                    .entry(cache_key)
                    .or_insert_with(async {
                        let res = validate(
                            &validation_schema.document,
                            &validation_operation,
                            &validation_plan,
                        );
                        Arc::new(res)
                    })
                    .await
                    .into_value_with_hit_miss(|r| match r {
                        CacheHitMiss::Hit => {
                            validate_span.record_cache_hit(true);
                            validate_cache_capture.finish_hit();
                        }
                        CacheHitMiss::Miss | CacheHitMiss::Error => {
                            cache_hint = CacheHint::Miss;
                            validate_span.record_cache_hit(false);
                            validate_cache_capture.finish_miss();
                        }
                    })
            }
        };

        if !on_end_callbacks.is_empty() {
            let mut end_payload = OnGraphQLValidationEndHookPayload { errors, cache_hint };
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
