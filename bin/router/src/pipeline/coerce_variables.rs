use std::collections::HashMap;

use hive_router_internal::telemetry::traces::spans::graphql::GraphQLVariableCoercionSpan;
use hive_router_plan_executor::execution::plan::CoerceVariablesPayload;
use hive_router_plan_executor::hooks::on_supergraph_load::SupergraphData;
use hive_router_plan_executor::variables::collect_variables;
use sonic_rs::Value;
use tracing::{trace, warn};

use crate::pipeline::error::PipelineError;
use crate::pipeline::normalize::GraphQLNormalizationPayload;

#[inline]
pub fn coerce_request_variables(
    supergraph: &SupergraphData,
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
            trace!(
                "sucessfully collected variables from incoming request: {:?}",
                values
            );

            Ok(CoerceVariablesPayload {
                variables_map: values,
            })
        }
        Err(err_msg) => {
            warn!(
                "failed to collect variables from incoming request: {}",
                err_msg
            );
            Err(PipelineError::VariablesCoercionError(err_msg))
        }
    }
}
