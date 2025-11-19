use std::collections::HashMap;
use std::sync::Arc;

use hive_router_plan_executor::hooks::on_graphql_params::GraphQLParams;
use hive_router_plan_executor::hooks::on_supergraph_load::SupergraphData;
use hive_router_plan_executor::variables::collect_variables;
use hive_router_query_planner::state::supergraph_state::OperationKind;
use http::Method;
use ntex::web::HttpRequest;
use sonic_rs::Value;
use tracing::{error, trace, warn};

use crate::pipeline::error::PipelineErrorVariant;
use crate::pipeline::normalize::GraphQLNormalizationPayload;

#[derive(Clone, Debug)]
pub struct CoerceVariablesPayload {
    pub variables_map: Option<HashMap<String, Value>>,
}

#[inline]
pub fn coerce_request_variables(
    req: &HttpRequest,
    supergraph: &SupergraphData,
    graphql_params: &mut GraphQLParams,
    normalized_operation: &Arc<GraphQLNormalizationPayload>,
) -> Result<CoerceVariablesPayload, PipelineErrorVariant> {
    if req.method() == Method::GET {
        if let Some(OperationKind::Mutation) =
            normalized_operation.operation_for_plan.operation_kind
        {
            error!("Mutation is not allowed over GET, stopping");

            return Err(PipelineErrorVariant::MutationNotAllowedOverHttpGet);
        }
    }

    match collect_variables(
        &normalized_operation.operation_for_plan,
        &mut graphql_params.variables,
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
            Err(PipelineErrorVariant::VariablesCoercionError(err_msg))
        }
    }
}
