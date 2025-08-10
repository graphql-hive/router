use std::collections::HashMap;
use std::sync::Arc;

use http::Method;
use ntex::web::HttpRequest;
use query_plan_executor::variables::collect_variables;
use query_planner::state::supergraph_state::OperationKind;
use serde_json::Value;
use tracing::{error, trace, warn};

use crate::pipeline::error::{PipelineError, PipelineErrorFromAcceptHeader, PipelineErrorVariant};
use crate::pipeline::graphql_request_params::ExecutionRequest;
use crate::pipeline::normalize_service::GraphQLNormalizationPayload;
use crate::shared_state::GatewaySharedState;

#[derive(Clone, Debug)]
pub struct CoerceVariablesPayload {
    pub variables_map: Option<HashMap<String, Value>>,
}

#[inline]
pub fn coerce_vars(
    req: &HttpRequest,
    execution_params: &ExecutionRequest,
    app_state: &GatewaySharedState,
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
        &execution_params.variables,
        &app_state.schema_metadata,
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
