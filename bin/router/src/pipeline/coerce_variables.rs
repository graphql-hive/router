use std::collections::HashMap;

use hive_router_internal::telemetry::traces::spans::graphql::GraphQLVariableCoercionSpan;
use hive_router_plan_executor::hooks::on_supergraph_load::SupergraphData;
use hive_router_plan_executor::variables::collect_variables;
use sonic_rs::{JsonValueTrait, Value};
use tracing::{trace, warn};

use crate::pipeline::error::PipelineError;
use crate::pipeline::normalize::GraphQLNormalizationPayload;

#[derive(Clone, Debug, Default)]
pub struct CoerceVariablesPayload {
    pub variables_map: Option<HashMap<String, Value>>,
}

impl CoerceVariablesPayload {
    pub fn variable_equals_true(&self, name: &str) -> bool {
        self.variables_map
            .as_ref()
            .and_then(|vars| vars.get(name))
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
    }
}

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
