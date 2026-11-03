use std::collections::HashMap;

use crate::executor::execution::plan::CoerceVariablesPayload;
use crate::executor::hooks::on_supergraph_load::SupergraphSnapshot;
use crate::executor::variables::{collect_variables_with_options, VariableCoercionError};
use crate::telemetry::logging::targets;
use crate::telemetry::traces::spans::graphql::GraphQLVariableCoercionSpan;
use sonic_rs::Value;
use tracing::{debug, warn};

use crate::pipeline::error::{ClientPipelineError, PipelineError};
use crate::pipeline::normalize::GraphQLNormalizationPayload;
use graphql_tools::validation::rules::ValidationOptions;

#[inline]
pub fn coerce_request_variables(
    supergraph: &SupergraphSnapshot,
    variables: &mut HashMap<String, Value>,
    normalized_operation: &GraphQLNormalizationPayload,
    validation_options: ValidationOptions,
) -> Result<CoerceVariablesPayload, PipelineError> {
    let span = GraphQLVariableCoercionSpan::new();
    let _guard = span.span.enter();
    // Goes into the request's own variables, as demand control and the rest of execution
    // only look there for values of `$_v0` and friends.
    variables.reserve(normalized_operation.extracted_variables.len());
    for extracted in normalized_operation.extracted_variables.iter() {
        variables.insert(extracted.name.clone(), (&extracted.value).into());
    }
    match collect_variables_with_options(
        &normalized_operation.operation_for_plan,
        variables,
        &supergraph.metadata,
        validation_options,
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
        Err(err) => {
            let err = point_at_argument(err, normalized_operation);
            warn!(
                target: targets::COERCE_VARIABLES,
                error = %err,
                "failed to collect variables from incoming request",
            );
            Err(ClientPipelineError::VariablesCoercionError(err).into())
        }
    }
}

/// Inline values are validated before they're turned into variables, so this shouldn't
/// happen. But if validation lets something through that coercion doesn't, the error names the
/// argument the client wrote, not `$_v0`.
fn point_at_argument(
    err: VariableCoercionError,
    normalized_operation: &GraphQLNormalizationPayload,
) -> VariableCoercionError {
    let VariableCoercionError::InvalidValue { name, path, reason } = err else {
        return err;
    };
    match normalized_operation
        .extracted_variables
        .iter()
        .find(|extracted| extracted.name == name)
    {
        Some(extracted) => VariableCoercionError::InvalidArgumentValue {
            argument: extracted.argument.clone(),
            path,
            reason,
        },
        None => VariableCoercionError::InvalidValue { name, path, reason },
    }
}
