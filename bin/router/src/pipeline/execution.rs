use std::collections::HashMap;
use std::sync::Arc;

use crate::pipeline::coerce_variables::CoerceVariablesPayload;
use crate::pipeline::error::PipelineErrorVariant;
use crate::pipeline::normalize::GraphQLNormalizationPayload;
use crate::shared_state::RouterSharedState;
use hive_router_plan_executor::execution::client_request_details::ClientRequestDetails;
use hive_router_plan_executor::execution::jwt_forward::JwtAuthForwardingPlan;
use hive_router_plan_executor::execution::plan::{PlanExecutionOutput, QueryPlanExecutionContext};
use hive_router_plan_executor::hooks::on_supergraph_load::SupergraphData;
use hive_router_plan_executor::introspection::resolve::IntrospectionContext;
use hive_router_plan_executor::plugin_context::PluginManager;
use hive_router_query_planner::planner::plan_nodes::QueryPlan;
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
pub async fn execute_plan<'exec, 'req>(
    req: &HttpRequest,
    supergraph: &SupergraphData,
    app_state: Arc<RouterSharedState>,
    normalized_payload: Arc<GraphQLNormalizationPayload>,
    query_plan_payload: Arc<QueryPlan>,
    variable_payload: &CoerceVariablesPayload,
    client_request_details: &ClientRequestDetails<'exec, 'req>,
    plugin_manager: PluginManager<'req>,
) -> Result<PlanExecutionOutput, PipelineErrorVariant> {
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

    let jwt_auth_forwarding: Option<JwtAuthForwardingPlan> = if app_state
        .router_config
        .jwt
        .is_jwt_extensions_forwarding_enabled()
    {
        client_request_details
            .jwt
            .build_forwarding_plan(
                &app_state
                    .router_config
                    .jwt
                    .forward_claims_to_upstream_extensions
                    .field_name,
            )
            .map_err(PipelineErrorVariant::JwtForwardingError)?
    } else {
        None
    };

    let ctx = QueryPlanExecutionContext {
        plugin_manager: &plugin_manager,
        query_plan: query_plan_payload,
        projection_plan: &normalized_payload.projection_plan,
        headers_plan: &app_state.headers_plan,
        variable_values: &variable_payload.variables_map,
        extensions,
        client_request: client_request_details,
        introspection_context: &introspection_context,
        operation_type_name: normalized_payload.root_type_name,
        jwt_auth_forwarding,
        executors: &supergraph.subgraph_executor_map,
    };

    ctx.execute_query_plan().await.map_err(|err| {
        tracing::error!("Failed to execute query plan: {}", err);
        PipelineErrorVariant::PlanExecutionError(err)
    })
}
