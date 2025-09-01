use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;

use crate::jwt::context::JwtRequestContext;
use crate::pipeline::coerce_variables::CoerceVariablesPayload;
use crate::pipeline::error::{PipelineError, PipelineErrorFromAcceptHeader, PipelineErrorVariant};
use crate::pipeline::normalize::GraphQLNormalizationPayload;
use crate::schema_state::SupergraphData;
use crate::shared_state::RouterSharedState;
use hive_router_plan_executor::execute_query_plan;
use hive_router_plan_executor::execution::jwt_forward::JwtAuthForwardingPlan;
use hive_router_plan_executor::execution::plan::{
    ClientRequestDetails, OperationDetails, PlanExecutionOutput, QueryPlanExecutionContext,
};
use hive_router_plan_executor::introspection::resolve::IntrospectionContext;
use hive_router_query_planner::planner::plan_nodes::QueryPlan;
use hive_router_query_planner::state::supergraph_state::OperationKind;
use http::HeaderName;
use ntex::web::HttpRequest;

static EXPOSE_QUERY_PLAN_HEADER: HeaderName = HeaderName::from_static("hive-expose-query-plan");

#[derive(Clone, Debug, PartialEq, Eq)]
enum ExposeQueryPlanMode {
    Yes,
    No,
    DryRun,
}

#[inline]
pub async fn execute_plan<'a>(
    req: &mut HttpRequest,
    query: Cow<'a, str>,
    supergraph: &SupergraphData,
    app_state: &Arc<RouterSharedState>,
    normalized_payload: &Arc<GraphQLNormalizationPayload>,
    query_plan_payload: &Arc<QueryPlan>,
    variable_payload: &CoerceVariablesPayload,
) -> Result<PlanExecutionOutput, PipelineError> {
    let mut expose_query_plan = ExposeQueryPlanMode::No;

    if app_state.router_config.query_planner.allow_expose {
        if let Some(expose_qp_header) = req.headers().get(&EXPOSE_QUERY_PLAN_HEADER) {
            let str_value = expose_qp_header.to_str().unwrap_or_default().trim();

            match str_value {
                "true" => expose_query_plan = ExposeQueryPlanMode::Yes,
                "dry-run" => expose_query_plan = ExposeQueryPlanMode::DryRun,
                _ => {}
            }
        }
    }

    let extensions = if expose_query_plan == ExposeQueryPlanMode::Yes
        || expose_query_plan == ExposeQueryPlanMode::DryRun
    {
        Some(HashMap::from_iter([(
            "queryPlan".to_string(),
            sonic_rs::to_value(&query_plan_payload).unwrap(),
        )]))
    } else {
        None
    };

    let introspection_context = IntrospectionContext {
        query: normalized_payload.operation_for_introspection.as_ref(),
        schema: &supergraph.planner.consumer_schema.document,
        metadata: &supergraph.metadata,
    };

    let jwt_context = {
        let req_extensions = req.extensions();
        req_extensions.get::<JwtRequestContext>().cloned()
    };
    let jwt_forward_plan: Option<JwtAuthForwardingPlan> = match jwt_context {
        Some(jwt_context) => jwt_context
            .try_into()
            .map_err(|e| req.new_pipeline_error(PipelineErrorVariant::JwtForwardingError(e)))?,
        None => None,
    };

    execute_query_plan(QueryPlanExecutionContext {
        query_plan: query_plan_payload,
        projection_plan: &normalized_payload.projection_plan,
        headers_plan: &app_state.headers_plan,
        variable_values: &variable_payload.variables_map,
        extensions,
        client_request: ClientRequestDetails {
            method: req.method().clone(),
            url: req.uri().clone(),
            headers: req.headers(),
            operation: OperationDetails {
                name: normalized_payload.operation_for_plan.name.clone(),
                kind: match normalized_payload.operation_for_plan.operation_kind {
                    Some(OperationKind::Query) => "query",
                    Some(OperationKind::Mutation) => "mutation",
                    Some(OperationKind::Subscription) => "subscription",
                    None => "query",
                },
                query,
            },
        },
        introspection_context: &introspection_context,
        operation_type_name: normalized_payload.root_type_name,
        jwt_auth_forwarding: &jwt_forward_plan,
        executors: &supergraph.subgraph_executor_map,
    })
    .await
    .map_err(|err| {
        tracing::error!("Failed to execute query plan: {}", err);
        req.new_pipeline_error(PipelineErrorVariant::PlanExecutionError(err))
    })
}
