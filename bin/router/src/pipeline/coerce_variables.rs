use std::collections::HashMap;
use std::sync::Arc;

use hive_router_plan_executor::variables::collect_variables;
use hive_router_query_planner::state::supergraph_state::OperationKind;
use http::Method;
use ntex::web::HttpRequest;
use sonic_rs::{JsonValueTrait, Value};
use tracing::{error, trace, warn};

use crate::pipeline::error::{PipelineError, PipelineErrorFromAcceptHeader, PipelineErrorVariant};
use crate::pipeline::execution_request::ExecutionRequest;
use crate::pipeline::normalize::GraphQLNormalizationPayload;
use crate::schema_state::SupergraphData;

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
    req: &HttpRequest,
    supergraph: &SupergraphData,
    execution_params: &mut ExecutionRequest,
    normalized_operation: &Arc<GraphQLNormalizationPayload>,
) -> Result<CoerceVariablesPayload, PipelineError> {
    if req.method() == Method::GET {
        if let Some(OperationKind::Mutation) =
            normalized_operation.operation_for_plan.operation_kind
        {
            error!("Mutation is not allowed over GET, stopping");

            return Err(req.new_pipeline_error(PipelineErrorVariant::MutationNotAllowedOverHttpGet));
        }
    }

    match collect_variables(
        &normalized_operation.operation_for_plan,
        &mut execution_params.variables,
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
            Err(req.new_pipeline_error(PipelineErrorVariant::VariablesCoercionError(err_msg)))
        }
    }
}
