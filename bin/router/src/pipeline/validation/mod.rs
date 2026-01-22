use std::sync::Arc;

use crate::pipeline::error::PipelineError;
use crate::pipeline::parser::GraphQLParserPayload;
use crate::schema_state::{SchemaState, SupergraphData};
use crate::shared_state::RouterSharedState;
use graphql_tools::validation::validate::validate;
use hive_router_internal::telemetry::traces::spans::graphql::GraphQLValidateSpan;
use tracing::{error, trace, Instrument};
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
    let validate_span = GraphQLValidateSpan::new();

    async {
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
                    consumer_schema_ast,
                    &parser_payload.parsed_operation,
                    &app_state.validation_plan,
                );
                let arc_res = Arc::new(res);

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

        Ok(())
    }
    .instrument(validate_span.clone())
    .await
}
