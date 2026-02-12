use std::sync::Arc;

use crate::cache_state::{CacheHitMiss, EntryValueHitMissExt};
use crate::pipeline::error::PipelineError;
use crate::pipeline::parser::GraphQLParserPayload;
use crate::schema_state::{SchemaState, SupergraphData};
use crate::shared_state::RouterSharedState;
use graphql_tools::validation::validate::validate;
use hive_router_internal::telemetry::traces::spans::graphql::GraphQLValidateSpan;
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
) -> Result<(), PipelineError> {
    let metrics = &schema_state.telemetry_context.metrics;
    let validate_cache_capture = metrics.cache.validate.capture_request();
    let validate_span = GraphQLValidateSpan::new();

    async {
        let consumer_schema_ast = &supergraph.planner.consumer_schema.document;

        let validation_result = schema_state
            .validate_cache
            .entry(parser_payload.cache_key)
            .or_insert_with(async {
                let res = validate(
                    consumer_schema_ast,
                    &parser_payload.parsed_operation,
                    &app_state.validation_plan,
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
                    validate_span.record_cache_hit(false);
                    validate_cache_capture.finish_miss();
                }
            });

        if !validation_result.is_empty() {
            error!(
                "GraphQL validation failed with total of {} errors",
                validation_result.len()
            );
            trace!("Validation errors: {:?}", validation_result);

            return Err(PipelineError::ValidationErrors(validation_result));
        }

        Ok(())
    }
    .instrument(validate_span.clone())
    .await
}
