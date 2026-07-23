use std::collections::HashMap;

use hive_router_internal::telemetry::logging::targets;
use hive_router_internal::telemetry::traces::spans::graphql::GraphQLVariableCoercionSpan;
use hive_router_plan_executor::execution::plan::CoerceVariablesPayload;
use hive_router_plan_executor::hooks::on_supergraph_load::SupergraphSnapshot;
use hive_router_plan_executor::variables::collect_variables;
use sonic_rs::Value;
use tracing::{debug, warn};

use crate::pipeline::error::PipelineError;
use crate::pipeline::normalize::GraphQLNormalizationPayload;

#[inline]
pub fn coerce_request_variables(
    supergraph: &SupergraphSnapshot,
    variables: &mut HashMap<String, Value>,
    normalized_operation: &GraphQLNormalizationPayload,
) -> Result<CoerceVariablesPayload, PipelineError> {
    let span = GraphQLVariableCoercionSpan::new();
    let _guard = span.span.enter();
    match collect_variables(
        &normalized_operation.operation_for_plan,
        variables,
        &supergraph.metadata,
    ) {
        Ok(values) => {
            debug!(
                target: targets::COERCE_VARIABLES,
                variables = ?values,
                "successfully collected variables from incoming request",
            );

            Ok(CoerceVariablesPayload {
                variables_map: values,
            })
        }
        Err(err_msg) => {
            warn!(
                target: targets::COERCE_VARIABLES,
                error = ?err_msg,
                "failed to collect variables from incoming request",
            );
            Err(PipelineError::VariablesCoercionError(err_msg))
        }
    }
}
